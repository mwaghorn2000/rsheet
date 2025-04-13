mod spreadsheet {
    pub mod spreadsheet;
}

use rsheet_lib::command::Command;
use rsheet_lib::connect::{
    Connection, Manager, ReadMessageResult, Reader, WriteMessageResult, Writer,
};
use rsheet_lib::replies::Reply;
use spreadsheet::spreadsheet::SpreadSheetHandler;

use std::error::Error;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use log::info;

pub fn start_server<M>(mut manager: M) -> Result<(), Box<dyn Error>>
where
    M: Manager,
{
    let spreadsheet = Arc::new(SpreadSheetHandler::new());
    let mut connections = Vec::new();
    loop {
        match manager.accept_new_connection() {
            Connection::NewConnection { reader, writer } => {
                let writer= Arc::new(Mutex::new(writer));
                let writer_clone = Arc::clone(&writer);
                let spreadsheet_clone = Arc::clone(&spreadsheet);
                connections.push(std::thread::spawn(move || {
                    handle_connection(reader, writer_clone, spreadsheet_clone)
                }));
            }
            Connection::NoMoreConnections => break,
        };
    }

    for handle in connections {
        let _ = handle.join();
    }

    Ok(())
}
fn handle_connection<R, W>(mut recv: R, mut send: Arc<Mutex<W>>, spreadsheet: Arc<SpreadSheetHandler>)
where
    R: Reader,
    W: Writer,
{
    loop {
        info!("Just got message from thread {:?}", thread::current().id());

        match recv.read_message() {
            ReadMessageResult::Message(msg) => {
                println!("Thread {:?} received message: {}", thread::current().id(), msg);
                let (tx, rx) = mpsc::channel();

                let reply: Option<Reply> = match msg.parse::<Command>() {
                    Ok(command) => {
                        println!(
                            "Thread {:?} parsed command: {}",
                            thread::current().id(),
                            msg
                        );

                        if let Err(e) = spreadsheet.send(command, tx) {
                            println!("Thread {:?} failed to send command to spreadsheet", thread::current().id());
                            Some(Reply::Error(format!(
                                "Failed to send command to spreadsheet: {}",
                                e
                            )))
                        } else {
                            println!("Thread {:?} waiting for reply", thread::current().id());
                            let response = rx.recv().unwrap_or(Some(Reply::Error(
                                "No response from spreadsheet".to_string(),
                            )));
                            println!(
                                "Thread {:?} got reply: {:?}",
                                thread::current().id(),
                                response
                            );
                            response
                        }
                    }
                    Err(e) => {
                        println!("Thread {:?} failed to parse command", thread::current().id());
                        Some(Reply::Error(e))
                    }
                };

                if let Some(reply) = reply {
                    match send.lock().unwrap().write_message(reply) {
                        WriteMessageResult::Ok => {
                            println!("Thread {:?} wrote message", thread::current().id());
                        }
                        WriteMessageResult::ConnectionClosed => {
                            println!("Thread {:?} connection closed", thread::current().id());
                            break;
                        }
                        WriteMessageResult::Err(_) => {
                            println!("Thread {:?} write error", thread::current().id());
                            return;
                        }
                    }
                } else {
                    println!("Thread {:?} got no reply to send", thread::current().id());
                }
            }
            ReadMessageResult::ConnectionClosed => {
                println!("Thread {:?} connection closed on read", thread::current().id());
                break;
            }
            ReadMessageResult::Err(_) => {
                println!("Thread {:?} read error", thread::current().id());
                return;
            }
        }
    }
}