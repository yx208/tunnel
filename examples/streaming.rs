#![allow(warnings, warnings)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use crossterm::terminal::{enable_raw_mode, disable_raw_mode};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};

use tunnel::config::get_config;
use tunnel::tus::client::{TusClient, UploadStrategy};
use tunnel::tus::errors::Result;
use tunnel::tus::manager::{UploadManager, UploadManagerHandle};
use tunnel::tus::types::UploadEvent;

#[tokio::main]
async fn main() -> Result<()> {
    let manager_handle = create_manager().await?;
    let manager = Arc::new(Mutex::new(manager_handle));
    
    let event_receiver = {
        let manger_guard = manager.lock().await;
        manger_guard.manager.subscribe_events()
    };
    let task_handle = tokio::spawn(handle_event(event_receiver));
    let keyboard_handle = tokio::spawn(handle_keyboard(manager));

    tokio::join!(task_handle, keyboard_handle);

    Ok(())
}

async fn handle_keyboard(manager_handle: Arc<Mutex<UploadManagerHandle>>) -> Result<()> {
    enable_raw_mode()?;

    loop {
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(KeyEvent { code, kind, .. }) = event::read()? {
                if kind != KeyEventKind::Press {
                    continue;
                }
                
                let handle = manager_handle.lock().await;

                match code {
                    KeyCode::Char('q') => {
                        println!("👋 Quitting...");
                        break;
                    }
                    KeyCode::Char('p') => {
                        let tasks = handle.manager.get_all_tasks().await?;
                        for task in tasks {
                            handle.manager.pause_upload(task.id).await?;
                        }
                    }
                    KeyCode::Char('r') => {
                        let tasks = handle.manager.get_all_tasks().await?;
                        for task in tasks {
                            handle.manager.resume_upload(task.id).await?;
                        }
                    }
                    KeyCode::Char('c') => {
                        let tasks = handle.manager.get_all_tasks().await?;
                        for task in tasks {
                            handle.manager.cancel_upload(task.id).await?;
                        }
                    }
                    KeyCode::Char('a') => {
                        let config = get_config();
                        let upload_file = PathBuf::from(&config.file_path);
                        handle.manager.add_upload(upload_file, None).await?;
                    }
                    KeyCode::Char('l') => {
                        let tasks = handle.manager.get_all_tasks().await?;
                        println!("============== All Task ==============");
                        for task in tasks {
                            println!("ID: {:?}, Status: {:?}", &task.id, &task.state);
                        }
                        println!("============== All Task ==============");
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    std::process::exit(0);
}

async fn handle_event(mut event_rx: broadcast::Receiver<UploadEvent>) {
    while let Ok(event) = event_rx.recv().await {
        match event {
            UploadEvent::Progress(progress) => {
                println!(
                    "Upload {:?}: {:.1}% ({:.2} MB/s), eta: {:?}",
                    progress.upload_id,
                    progress.percentage,
                    progress.instant_speed / (1024.0 * 1024.0),
                    progress.eta
                );
            }
            UploadEvent::StateChanged { upload_id, old_state, new_state } => {
                println!("Upload {:?}: {:?} -> {:?}", upload_id, old_state, new_state);
            }
            UploadEvent::Completed { upload_id, upload_url } => {
                println!("Upload {:?}: completed: {}", upload_id, upload_url);
            }
            UploadEvent::Failed { upload_id, error } => {
                println!("Upload {:?} failed: {}", upload_id, error);
            }
        }
    }
}

async fn create_manager() -> Result<UploadManagerHandle> {
    let config = get_config();
    let client = TusClient::new(&config.endpoint, 1024 * 1024 * 10);

    Ok(UploadManager::new(client, 3, None))
}
