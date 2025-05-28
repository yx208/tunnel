#![allow(warnings, warnings)]

use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::broadcast;
use tunnel::config::get_config;
use tunnel::tus::client::{TusClient, UploadStrategy};
use tunnel::tus::errors::Result;
use tunnel::tus::manager::{UploadManager, UploadManagerHandle};
use tunnel::tus::types::UploadEvent;
use crossterm::terminal::{enable_raw_mode, disable_raw_mode};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};

#[tokio::main]
async fn main() -> Result<()> {
    let manager_handle = create_manager().await?;

    // 接收消息
    let events = manager_handle.manager.subscribe_events();
    let task_handle = tokio::spawn(handle_event(events));
    let keyboard_handle = tokio::spawn(handle_keyboard(manager_handle));

    tokio::join!(task_handle, keyboard_handle);

    Ok(())
}

async fn handle_keyboard(manager_handle: UploadManagerHandle) -> Result<()> {
    enable_raw_mode()?;

    loop {
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(KeyEvent { code, kind, .. }) = event::read()? {
                if kind != KeyEventKind::Press {
                    continue;
                }

                match code {
                    KeyCode::Char('q') => {
                        println!("👋 Quitting...");
                        std::process::exit(0);
                    }
                    KeyCode::Char('p') => {
                        let tasks = manager_handle.manager.get_all_tasks().await?;
                        for task in tasks {
                            manager_handle.manager.pause_upload(task.id).await?;
                        }
                    }
                    KeyCode::Char('r') => {
                        let tasks = manager_handle.manager.get_all_tasks().await?;
                        for task in tasks {
                            manager_handle.manager.resume_upload(task.id).await?;
                        }
                    }
                    KeyCode::Char('c') => {
                        let tasks = manager_handle.manager.get_all_tasks().await?;
                        for task in tasks {
                            manager_handle.manager.cancel_upload(task.id).await?;
                        }
                    }
                    KeyCode::Char('a') => {
                        let config = get_config();
                        let upload_file = PathBuf::from(&config.file_path);
                        manager_handle.manager.add_upload(upload_file, None).await?;
                    }
                    KeyCode::Char('l') => {
                        let tasks = manager_handle.manager.get_all_tasks().await?;
                        println!("============== All Tasks ==============");
                        for task in tasks {
                            println!("ID: {:?}, Status: {:?}", &task.id, &task.state);
                        }
                        println!("============== All Tasks ==============");
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn handle_event(mut event_rx: broadcast::Receiver<UploadEvent>) {
    while let Ok(event) = event_rx.recv().await {
        match event {
            UploadEvent::Progress(progress) => {
                println!(
                    "Upload {:?}: {:.1}% ({:.2} MB/s)",
                    progress.upload_id,
                    progress.percentage,
                    progress.instant_speed / (1024.0 * 1024.0)
                );
            }
            UploadEvent::StateChanged { upload_id, old_state, new_state } => {
                println!("Upload {:?}: {:?} -> {:?}", upload_id, old_state, new_state);
            }
            UploadEvent::Completed { upload_id, upload_url } => {
                println!("Upload {:?} completed: {}", upload_id, upload_url);
            }
            UploadEvent::Failed { upload_id, error } => {
                println!("Upload {:?} failed: {}", upload_id, error);
            }
        }
    }
}

async fn create_manager() -> Result<UploadManagerHandle> {
    let config = get_config();
    let client = TusClient::with_strategy(
        &config.endpoint,
        1024 * 1024 * 10,
        UploadStrategy::Streaming
    );

    Ok(UploadManager::new(client, 3, None))
}
