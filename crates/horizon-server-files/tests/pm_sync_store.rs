#![cfg(feature = "pm-sync-mysql")]

use horizon_protocol::crypto::uuid_string;
use horizon_server_files::pm_sync_store::{
    Checkpoint, CheckpointDisposition, Handover, HandoverOutcome, Store, StoreError,
};
use mysql::{Conn, Opts, prelude::Queryable};
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
    let raw = include_str!("fixtures/pm-sync/java-v2.hex")
        .lines()
        .find_map(|line| line.strip_prefix("canonical "))
        .unwrap();
    let snapshot: Vec<u8> = (0..raw.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&raw[at..at + 2], 16).unwrap())
        .collect();
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
