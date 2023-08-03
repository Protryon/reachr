use std::{collections::HashMap, time::Duration};

use always_cell::AlwaysCell;
use config::{Config, CONFIG, CONFIG_FILE};
use log::info;
use really_notify::FileWatcherConfig;
use tokio::{select, sync::watch, time::MissedTickBehavior};
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::config::Target;

mod config;
mod testing;

#[tokio::main]
async fn main() {
    env_logger::Builder::new()
        .parse_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    let mut config_update_receiver = FileWatcherConfig::new(&*CONFIG_FILE, "config")
        .with_parser(|data| serde_yaml::from_slice::<Config>(&data))
        .start();

    info!("loading config...");
    let (config_sender, config_receiver) = watch::channel(
        config_update_receiver
            .recv()
            .await
            .expect("missing initial config"),
    );
    AlwaysCell::set(&CONFIG, config_receiver);
    tokio::spawn(async move {
        while let Some(update) = config_update_receiver.recv().await {
            config_sender.send_replace(update);
        }
    });

    prometheus_exporter::start(CONFIG.borrow().bind).expect("failed to start prometheus exporter");

    let mut tasks: HashMap<Target, DropGuard> = HashMap::new();

    let mut watcher = CONFIG.clone();

    loop {
        {
            let config = watcher.borrow_and_update();
            for target in config.targets.iter() {
                if tasks.contains_key(target) {
                    continue;
                }
                let guard = CancellationToken::new();
                let handle = guard.clone().drop_guard();
                let target2 = target.clone();

                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(
                        target2.interval.unwrap_or(CONFIG.borrow().interval),
                    ));
                    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
                    loop {
                        select! {
                            _ = guard.cancelled() => {
                                target2.remove();
                                break;
                            },
                            _ = interval.tick() => {
                                target2.test().await;
                            },
                        }
                    }
                });

                tasks.insert(target.clone(), handle);
            }
            // remove tasks
            tasks.retain(|target, _| config.targets.contains(target));
        }
        if watcher.changed().await.is_err() {
            return;
        }
    }
}
