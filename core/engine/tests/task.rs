// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The per-account task: wake-ups, the periodic sync, parallel accounts and shutdown. These run on
//! tokio's paused clock (`start_paused`), and the engine reads the same clock, so no test sleeps
//! in real time and none depends on how fast the machine is.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{create_account, Call, FakeBackend, Method};
use croft_engine::{AccountId, Clock, Engine, EngineConfig, EngineHandle};
use croft_protocols::{FlagChange, Flags};
use croft_storage::{folders, messages, Db};

/// Unix milliseconds that move with tokio's (paused) clock.
struct TokioClock {
    start: tokio::time::Instant,
}

impl Clock for TokioClock {
    fn now_ms(&self) -> i64 {
        1_000_000 + i64::try_from(self.start.elapsed().as_millis()).unwrap()
    }
}

struct Setup {
    engine: Engine,
    ui: EngineHandle,
    clock: Arc<TokioClock>,
    account: AccountId,
    backend: Arc<FakeBackend>,
}

fn setup(config: EngineConfig) -> Setup {
    let db = Db::open_in_memory().unwrap();
    let account = create_account(&db, "one@example.org");
    let clock = Arc::new(TokioClock {
        start: tokio::time::Instant::now(),
    });
    let backend = FakeBackend::new(clock.clone());
    backend.add("INBOX", "a@x", Flags::default());
    let engine = Engine::open(db, clock.clone(), config).unwrap();
    let ui = engine.handle();
    Setup {
        engine,
        ui,
        clock,
        account,
        backend,
    }
}

/// Lets the paused runtime run until `cond` holds (each step moves the paused clock by 10 ms).
async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..100_000 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {what}");
}

fn row_id(ui: &EngineHandle, account: AccountId, folder: &str, uid: u32) -> Option<i64> {
    ui.read(|db| {
        let f = folders::find_by_path(db.connection(), account, folder).unwrap()?;
        messages::find_by_uid(db.connection(), account, f.id, uid)
            .unwrap()
            .map(|m| m.id)
    })
}

fn count(backend: &FakeBackend, f: impl Fn(&Call) -> bool) -> usize {
    backend.calls().iter().filter(|c| f(c)).count()
}

#[tokio::test(start_paused = true)]
async fn the_task_syncs_first_then_sends_a_change_when_due_and_stops_on_shutdown() {
    let s = setup(EngineConfig::default());
    s.engine.add_account(s.account, s.backend.clone()).unwrap();
    until("first sync", || {
        row_id(&s.ui, s.account, "INBOX", 1).is_some()
    })
    .await;

    let id = row_id(&s.ui, s.account, "INBOX", 1).unwrap();
    let made = s.clock.now_ms();
    s.ui.mark_seen(s.account, id, true).unwrap();
    until("the change is sent", || {
        count(&s.backend, |c| matches!(c, Call::SetFlag(..))) == 1
    })
    .await;
    let sent = s
        .backend
        .timed_calls()
        .into_iter()
        .find(|(_, c)| matches!(c, Call::SetFlag(..)))
        .unwrap();
    assert_eq!(
        sent.1,
        Call::SetFlag("INBOX".into(), vec![1], FlagChange::Seen(true))
    );
    assert!(
        sent.0 >= made + 1_500,
        "not before its 1.5 s delay: {}",
        sent.0 - made
    );
    assert!(sent.0 < made + 1_600, "woken by the due time, not a poll");

    tokio::time::timeout(Duration::from_secs(1), s.engine.shutdown())
        .await
        .expect("shutdown returns promptly");
    // After shutdown, actions still work locally and nothing is sent.
    s.ui.mark_seen(s.account, id, false).unwrap();
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(count(&s.backend, |c| matches!(c, Call::SetFlag(..))), 1);
}

#[tokio::test(start_paused = true)]
async fn the_task_syncs_on_its_timer_and_at_once_on_sync_now() {
    let s = setup(EngineConfig {
        sync_interval_ms: 60_000,
        ..EngineConfig::default()
    });
    let lists = |b: &FakeBackend| count(b, |c| *c == Call::ListFolders);
    s.engine.add_account(s.account, s.backend.clone()).unwrap();
    until("timer sync", || lists(&s.backend) == 2).await;
    let times: Vec<i64> = s
        .backend
        .timed_calls()
        .into_iter()
        .filter(|(_, c)| *c == Call::ListFolders)
        .map(|(t, _)| t)
        .collect();
    let (first, second) = (times[0], times[1]);
    assert!(
        (60_000..60_100).contains(&(second - first)),
        "{}",
        second - first
    );

    let asked = s.clock.now_ms();
    s.ui.sync_now(s.account);
    until("sync now", || lists(&s.backend) == 3).await;
    assert!(s.clock.now_ms() - asked < 100);
    s.engine.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn an_offline_account_waits_for_a_poke_and_reconnecting_sends_what_waited() {
    let s = setup(EngineConfig::default());
    s.engine.add_account(s.account, s.backend.clone()).unwrap();
    until("first sync", || {
        row_id(&s.ui, s.account, "INBOX", 1).is_some()
    })
    .await;
    s.ui.set_online(s.account, false);
    let id = row_id(&s.ui, s.account, "INBOX", 1).unwrap();
    s.ui.set_flagged(s.account, id, true).unwrap();
    s.backend.clear_calls();
    tokio::time::sleep(Duration::from_secs(3 * 3_600)).await;
    assert!(s.backend.calls().is_empty(), "no timer runs while offline");
    s.ui.set_online(s.account, true);
    until("sent after reconnecting", || {
        s.backend.writes()
            == vec![Call::SetFlag(
                "INBOX".into(),
                vec![1],
                FlagChange::Flagged(true),
            )]
    })
    .await;
    s.engine.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_hung_server_blocks_neither_the_ui_nor_another_account_nor_shutdown() {
    let s = setup(EngineConfig {
        shutdown_grace_ms: 2_000,
        ..EngineConfig::default()
    });
    // Account 2 on its own server, which works.
    let other = s.ui.read(|db| create_account(db, "two@example.org"));
    let other_backend = FakeBackend::new(s.clock.clone());
    other_backend.add("INBOX", "b@x", Flags::default());
    // Account 1's server never answers a folder list.
    let _gate = s.backend.gate(Method::ListFolders);
    s.engine.add_account(s.account, s.backend.clone()).unwrap();
    s.engine.add_account(other, other_backend.clone()).unwrap();
    until("account 2 synced", || {
        row_id(&s.ui, other, "INBOX", 1).is_some()
    })
    .await;
    assert!(
        row_id(&s.ui, s.account, "INBOX", 1).is_none(),
        "account 1 is stuck"
    );

    // The UI path does not wait for the stuck account.
    let id = row_id(&s.ui, other, "INBOX", 1).unwrap();
    s.ui.mark_seen(other, id, true).unwrap();
    until("account 2's change sent", || {
        other_backend.writes().len() == 1
    })
    .await;

    // Shutdown stops the stuck task after the grace period.
    tokio::time::timeout(Duration::from_secs(10), s.engine.shutdown())
        .await
        .expect("shutdown does not hang on a stuck server");
}

#[tokio::test(start_paused = true)]
async fn removing_an_account_stops_its_task_and_keeps_its_queue() {
    let s = setup(EngineConfig::default());
    s.engine.add_account(s.account, s.backend.clone()).unwrap();
    until("first sync", || {
        row_id(&s.ui, s.account, "INBOX", 1).is_some()
    })
    .await;
    let id = row_id(&s.ui, s.account, "INBOX", 1).unwrap();
    s.engine.remove_account(s.account).await;
    s.ui.set_flagged(s.account, id, true).unwrap();
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(s.backend.writes().is_empty());
    // Added back, it sends what waited.
    s.engine.add_account(s.account, s.backend.clone()).unwrap();
    until("sent", || s.backend.writes().len() == 1).await;
    s.engine.shutdown().await;
}

#[tokio::test]
async fn add_account_needs_a_runtime_and_refuses_a_second_backend() {
    let s = setup(EngineConfig::default());
    s.engine
        .attach_account(s.account, s.backend.clone())
        .unwrap();
    assert!(matches!(
        s.engine.attach_account(s.account, s.backend.clone()),
        Err(croft_engine::EngineError::AccountExists(_))
    ));
    assert!(matches!(
        s.ui.run_once(99, false).await,
        Err(croft_engine::EngineError::UnknownAccount(99))
    ));
    let handle = s.ui.clone();
    let account = s.account;
    std::thread::spawn(move || {
        let db = Db::open_in_memory().unwrap();
        let engine = Engine::open(
            db,
            Arc::new(croft_engine::SystemClock),
            EngineConfig::default(),
        )
        .unwrap();
        let backend = FakeBackend::new(Arc::new(croft_engine::SystemClock));
        assert!(matches!(
            engine.add_account(account, backend),
            Err(croft_engine::EngineError::NoRuntime)
        ));
        drop(handle);
    })
    .join()
    .unwrap();
}

#[test]
fn the_handle_is_send_sync_and_cheap_to_clone() {
    fn assert_send_sync<T: Send + Sync + Clone>() {}
    assert_send_sync::<EngineHandle>();
}
