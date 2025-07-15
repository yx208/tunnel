use tunnel::{builder, presets};
use tunnel::config::get_config;
use tunnel::core::{TransferEvent, TransferManager};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let env_config = get_config();

    let config = presets::performance_config();
    let manger_handle = TransferManager::new(config);
    let manager = manger_handle.manager.clone();

    let mut event_rx = manager.subscribe();
    let event_handle = tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            handle_event(event);
        }
    });

    // Ensure upload file
    // ...

    manager.add_task(Box::new(
        builder::upload_tus(env_config.file_path, env_config.endpoint)
    )).await?;

    tokio::join!(event_handle);

    Ok(())
}

fn handle_event(event: TransferEvent) {
    match event {
        TransferEvent::StateChange { id, old_state, new_state } => {
            println!("[{}] State: {:?} -> {:?}", id, old_state, new_state);
        }

        TransferEvent::Progress { id, stats } => {
            let percentage = stats.progress_percentage();
            let speed_mb = stats.current_speed / 1024.0 / 1024.0;
            println!("[{}] Progress: {:.1}% @ {:.2} MB/s", id, percentage, speed_mb);
        }

        TransferEvent::BatchProgress { updates } => {
            for (id, stats) in updates {
                let percentage = stats.progress_percentage();
                let speed_mb = stats.current_speed / 1024.0 / 1024.0;
                println!("  [{}] {:.1}% @ {:.2} MB/s", id, percentage, speed_mb);
            }
        }

        TransferEvent::Completed { id, stats } => {
            let duration = stats.elapsed();
            let avg_speed_mb = stats.average_speed / 1024.0 / 1024.0;
            println!(
                "[{}] Completed! {} bytes in {:?} (avg {:.2} MB/s)",
                id, stats.bytes_transferred, duration, avg_speed_mb
            );
        }

        TransferEvent::Failed { id, error, .. } => {
            println!("[{}] Failed: {}", id, error);
        }

        TransferEvent::Retry { id, attempt, reason } => {
            println!("[{}] Retry attempt {}: {}", id, attempt, reason);
        }

        _ => {}
    }
}
