#![cfg(feature = "pm-sync-mysql")]

use horizon_protocol::crypto::uuid_string;
use horizon_server_files::pm_sync_store::{Handover, HandoverOutcome, Store, StoreError};
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
