#![cfg(feature = "pm-sync-mysql")]

use horizon_protocol::crypto::uuid_string;
use horizon_server_files::pm_sync_store::{
    Checkpoint, CheckpointDisposition, Handover, HandoverOutcome, ReleasedClaim, Store, StoreError,
};
use mysql::{Conn, Opts, prelude::Queryable};
use sha2::{Digest, Sha256};
use std::sync::Barrier;

const PLAYER: u128 = 0xa1;
const PAPER: u128 = 0xa2;
const EAST: u128 = 0xa3;
const WEST: u128 = 0xa4;
const NORTH: u128 = 0xa5;
const PREFIX: &str = "pigeon_qa_";
const MAX_BYTES: usize = 16 << 20;

fn connect() -> Conn {
    let url = std::env::var("PIGEON_PM_SYNC_TEST_DATABASE_URL")
        .expect("point the test at its disposable MariaDB database");
    let opts = Opts::from_url(&url).expect("invalid test database URL");
    assert_eq!(opts.get_db_name(), Some("pigeon_pmsync_test"));
    Conn::new(opts).expect("disposable test database is unavailable")
}

fn setup() -> Store {
    Store::open(connect(), PREFIX, MAX_BYTES).unwrap()
}

fn java_snapshot() -> Vec<u8> {
    let raw = include_str!("fixtures/pm-sync/java-v2.hex")
        .lines()
        .find_map(|line| line.strip_prefix("canonical "))
        .unwrap();
    (0..raw.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&raw[at..at + 2], 16).unwrap())
        .collect()
}

#[test]
#[ignore = "requires isolated MariaDB seeded by the original Java SqlSnapshotStore"]
fn java_lease_moves_once_with_conflict_and_transaction_failure_recovery() {
    let mut store = setup();
    let lease = store.acquire(PLAYER, PAPER, "paper").unwrap();
    assert_eq!(lease.revision, 1);
    let snapshot = lease.snapshot.unwrap();
    let incoming = Handover {
        player: PLAYER,
        request: 0xb1,
        source: "paper",
        source_token: PAPER,
        source_revision: 1,
        destination: "east",
        destination_token: EAST,
        snapshot: &snapshot,
    };
    assert_eq!(
        store.handover(&incoming).unwrap(),
        HandoverOutcome::Committed { revision: 2 }
    );
    assert_eq!(
        setup().handover(&incoming).unwrap(),
        HandoverOutcome::CurrentRetry { revision: 2 }
    );
    assert_eq!(
        setup()
            .acquire(PLAYER, EAST, "east")
            .unwrap()
            .snapshot
            .as_deref(),
        Some(snapshot.as_slice())
    );
    assert!(matches!(
        setup().acquire(PLAYER, PAPER, "paper"),
        Err(StoreError::Fenced)
    ));
    assert!(matches!(
        store.handover(&Handover {
            destination: "other",
            ..incoming
        }),
        Err(StoreError::Conflict)
    ));

    let mut sql = connect();
    sql.query_drop(format!(
        "CREATE TRIGGER `{PREFIX}fail_receipt` BEFORE INSERT ON `{PREFIX}checkpoint_receipts` \
         FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='fixture refuses the final receipt'"
    ))
    .unwrap();
    let outgoing = Handover {
        player: PLAYER,
        request: 0xb2,
        source: "east",
        source_token: EAST,
        source_revision: 2,
        destination: "west",
        destination_token: WEST,
        snapshot: &snapshot,
    };
    assert!(matches!(store.handover(&outgoing), Err(StoreError::Sql(_))));
    assert_eq!(setup().acquire(PLAYER, EAST, "east").unwrap().revision, 2);
    let count: Option<i64> = sql
        .exec_first(
            format!("SELECT COUNT(*) FROM `{PREFIX}history` WHERE player_uuid=? AND revision=3"),
            (uuid_string(PLAYER),),
        )
        .unwrap();
    assert_eq!(count, Some(0));
    sql.query_drop(format!("DROP TRIGGER `{PREFIX}fail_receipt`"))
        .unwrap();

    let simultaneous = Barrier::new(2);
    let (west, north) = std::thread::scope(|scope| {
        let west = scope.spawn(|| {
            let mut store = setup();
            simultaneous.wait();
            store.handover(&outgoing)
        });
        let north = scope.spawn(|| {
            let mut store = setup();
            simultaneous.wait();
            store.handover(&Handover {
                request: 0xb3,
                destination: "north",
                destination_token: NORTH,
                ..outgoing
            })
        });
        (west.join().unwrap(), north.join().unwrap())
    });
    let landed = match (west, north) {
        (Ok(HandoverOutcome::Committed { revision: 3 }), Err(StoreError::Fenced)) => (WEST, "west"),
        (Err(StoreError::Fenced), Ok(HandoverOutcome::Committed { revision: 3 })) => {
            (NORTH, "north")
        }
        _ => panic!("exactly one backend should have acquired revision three"),
    };
    assert_eq!(
        setup()
            .acquire(PLAYER, landed.0, landed.1)
            .unwrap()
            .revision,
        3
    );
    assert_eq!(
        setup().handover(&incoming).unwrap(),
        HandoverOutcome::Historical { revision: 2 }
    );
}

#[test]
#[ignore = "requires an empty checkpoint schema created by the original Java SqlSnapshotStore"]
fn native_checkpoint_and_release_leave_the_exact_snapshot_for_paper() {
    const SCHEMA: &str = "pigeon_checkpoint_";
    const CHECKPOINT_PLAYER: u128 = 0xa6;
    let open = || Store::open(connect(), SCHEMA, MAX_BYTES).unwrap();
    let mut store = open();
    let snapshot = java_snapshot();
    assert_eq!(
        store
            .acquire(CHECKPOINT_PLAYER, EAST, "east")
            .unwrap()
            .revision,
        0
    );
    let initial = Checkpoint {
        player: CHECKPOINT_PLAYER,
        request: 0xb4,
        backend: "east",
        token: EAST,
        revision: 0,
        disposition: CheckpointDisposition::KeepLease,
        snapshot: &snapshot,
    };
    assert_eq!(store.checkpoint_receipt(&initial).unwrap(), None);
    assert_eq!(store.checkpoint(&initial).unwrap(), 1);
    assert_eq!(open().checkpoint_receipt(&initial).unwrap(), Some(1));
    assert!(matches!(
        open().checkpoint(&initial),
        Err(StoreError::DuplicateCheckpoint)
    ));
    assert!(matches!(
        store.checkpoint(&Checkpoint {
            disposition: CheckpointDisposition::ReleaseLease,
            ..initial
        }),
        Err(StoreError::Conflict)
    ));
    assert!(matches!(
        open().acquire(CHECKPOINT_PLAYER, PAPER, "paper"),
        Err(StoreError::Fenced)
    ));
    let release = Checkpoint {
        request: 0xb5,
        revision: 1,
        disposition: CheckpointDisposition::ReleaseLease,
        ..initial
    };
    let mut sql = connect();
    sql.query_drop(format!(
        "CREATE TRIGGER `{SCHEMA}fail_receipt` BEFORE INSERT ON `{SCHEMA}checkpoint_receipts` \
         FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='release receipt fault'"
    ))
    .unwrap();
    assert!(matches!(
        store.checkpoint(&release),
        Err(StoreError::Sql(_))
    ));
    assert_eq!(
        open()
            .acquire(CHECKPOINT_PLAYER, EAST, "east")
            .unwrap()
            .revision,
        1
    );
    assert_eq!(open().checkpoint_receipt(&release).unwrap(), None);
    sql.query_drop(format!("DROP TRIGGER `{SCHEMA}fail_receipt`"))
        .unwrap();
    assert_eq!(store.checkpoint(&release).unwrap(), 2);
    assert_eq!(open().checkpoint_receipt(&release).unwrap(), Some(2));
    assert!(matches!(
        open().checkpoint(&release),
        Err(StoreError::DuplicateCheckpoint)
    ));
    assert!(matches!(
        open().checkpoint(&Checkpoint {
            request: 0xb6,
            revision: 2,
            ..release
        }),
        Err(StoreError::Fenced)
    ));
    let row: Option<(i64, Option<String>, Option<String>, Vec<u8>)> = sql
        .exec_first(
            format!("SELECT revision,owner_token,owner_server,snapshot FROM `{SCHEMA}state` WHERE player_uuid=?"),
            (uuid_string(CHECKPOINT_PLAYER),),
        )
        .unwrap();
    assert_eq!(row, Some((2, None, None, snapshot)));
}

#[test]
#[ignore = "requires a released snapshot seeded by the original Java SqlSnapshotStore"]
fn paper_release_can_only_be_claimed_by_one_exact_route() {
    const CLAIM_SCHEMA: &str = "pigeon_claim_";
    const PLAYER: u128 = 0xa7;
    let open = || Store::open(connect(), CLAIM_SCHEMA, MAX_BYTES).unwrap();
    let mut store = open();
    let snapshot = java_snapshot();
    let east = ReleasedClaim {
        player: PLAYER,
        request: 0xb7,
        source: "paper",
        backend: "east",
        token: EAST,
        revision: 1,
        snapshot_sha256: Sha256::digest(&snapshot).into(),
    };
    for wrong in [
        ReleasedClaim {
            player: PLAYER + 1,
            ..east
        },
        ReleasedClaim {
            revision: 2,
            ..east
        },
        ReleasedClaim {
            snapshot_sha256: [0; 32],
            ..east
        },
    ] {
        assert!(matches!(
            store.claim_released_snapshot(&wrong),
            Err(StoreError::Fenced)
        ));
    }
    assert_eq!(
        store.current_owner_revision(PLAYER, EAST, "east").unwrap(),
        None
    );
    assert!(matches!(
        store.with_released_claim(&east, || panic!("the release has not been claimed")),
        Err(StoreError::Fenced)
    ));
    let mut sql = connect();
    sql.query_drop(format!(
        "CREATE TRIGGER `{CLAIM_SCHEMA}fail_receipt` BEFORE INSERT ON `{CLAIM_SCHEMA}checkpoint_receipts` \
         FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='claim receipt fault'"
    )).unwrap();
    assert!(matches!(
        store.claim_released_snapshot(&east),
        Err(StoreError::Sql(_))
    ));
    assert_eq!(
        store.current_owner_revision(PLAYER, EAST, "east").unwrap(),
        None
    );
    sql.query_drop(format!("DROP TRIGGER `{CLAIM_SCHEMA}fail_receipt`"))
        .unwrap();
    let west = ReleasedClaim {
        request: 0xb8,
        backend: "west",
        token: WEST,
        ..east
    };
    let simultaneous = Barrier::new(2);
    let (a, b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            let mut store = open();
            simultaneous.wait();
            store.claim_released_snapshot(&east)
        });
        let b = scope.spawn(|| {
            let mut store = open();
            simultaneous.wait();
            store.claim_released_snapshot(&west)
        });
        (a.join().unwrap(), b.join().unwrap())
    });
    let (winner, lease) = match (a, b) {
        (Ok(lease), Err(StoreError::Fenced)) => (&east, lease),
        (Err(StoreError::Fenced), Ok(lease)) => (&west, lease),
        _ => panic!("exactly one replica must claim the original Paper release"),
    };
    assert_eq!(lease.revision, 1);
    assert_eq!(lease.snapshot.as_deref(), Some(snapshot.as_slice()));
    assert_eq!(open().claim_released_snapshot(winner).unwrap().revision, 1);
    let mut contender = connect();
    contender
        .query_drop("SET SESSION innodb_lock_wait_timeout=1")
        .unwrap();
    let checkpoint_lock =
        format!("SELECT revision FROM `{CLAIM_SCHEMA}state` WHERE player_uuid=? FOR UPDATE");
    assert_eq!(
        open()
            .with_released_claim(winner, || {
                let mut raced = contender
                    .start_transaction(mysql::TxOpts::default())
                    .unwrap();
                let blocked: Result<Option<i64>, _> =
                    raced.exec_first(&checkpoint_lock, (uuid_string(PLAYER),));
                assert!(
                    matches!(blocked, Err(mysql::Error::MySqlError(error)) if error.code == 1205)
                );
                raced.rollback().unwrap();
                "engine receipt"
            })
            .unwrap(),
        "engine receipt"
    );
    // End only this disposable SQL session after it checks the locked claim. The callback
    // completed, but the engine must still wait for an acknowledged transaction result.
    let mut connection = connect();
    let connection_id: u64 = connection
        .query_first("SELECT CONNECTION_ID()")
        .unwrap()
        .unwrap();
    let mut interrupted = Store::open(connection, CLAIM_SCHEMA, MAX_BYTES).unwrap();
    assert!(matches!(
        interrupted.with_released_claim(winner, || {
            connect()
                .query_drop(format!("KILL CONNECTION {connection_id}"))
                .unwrap();
        }),
        Err(StoreError::Sql(_))
    ));
    assert!(matches!(
        open().acquire(PLAYER, NORTH, "north"),
        Err(StoreError::Fenced)
    ));
    let url = std::env::var("PIGEON_PM_SYNC_TEST_DATABASE_URL").unwrap();
    Store::connect_authority(&url, CLAIM_SCHEMA, MAX_BYTES)
        .unwrap()
        .with_released_claim(winner, || ())
        .unwrap();
    assert!(matches!(
        open().claim_released_snapshot(&ReleasedClaim {
            source: "other-paper",
            ..*winner
        }),
        Err(StoreError::Conflict)
    ));
    assert_eq!(
        open()
            .current_owner_revision(PLAYER, winner.token, winner.backend)
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        open()
            .current_owner_revision(PLAYER, winner.token, &winner.backend.to_ascii_uppercase())
            .unwrap(),
        None
    );
    let history: Option<i32> = sql
        .exec_first(
            format!("SELECT COUNT(*) FROM `{CLAIM_SCHEMA}history` WHERE player_uuid=?"),
            (uuid_string(PLAYER),),
        )
        .unwrap();
    assert_eq!(history, Some(1));
    assert_eq!(
        store
            .checkpoint(&Checkpoint {
                player: PLAYER,
                request: 0xb9,
                token: winner.token,
                backend: winner.backend,
                revision: 1,
                disposition: CheckpointDisposition::KeepLease,
                snapshot: &snapshot,
            })
            .unwrap(),
        2
    );
    assert!(matches!(
        open().claim_released_snapshot(winner),
        Err(StoreError::Fenced)
    ));
    assert!(matches!(
        open().with_released_claim(winner, || panic!("the player has a newer revision")),
        Err(StoreError::Fenced)
    ));
    assert_eq!(
        open()
            .current_owner_revision(PLAYER, winner.token, winner.backend)
            .unwrap(),
        Some(2)
    );
    assert_eq!(
        open()
            .current_owner_revision(PLAYER, PAPER, "paper")
            .unwrap(),
        None
    );
}
