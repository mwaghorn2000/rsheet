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
                let writer = Arc::new(Mutex::new(writer));
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
fn handle_connection<R, W>(
    mut recv: R,
    mut send: Arc<Mutex<W>>,
    spreadsheet: Arc<SpreadSheetHandler>,
) where
    R: Reader,
    W: Writer,
{
    loop {
        info!("Just got message from thread {:?}", thread::current().id());
        match recv.read_message() {
            ReadMessageResult::Message(msg) => {
                let (tx, rx) = mpsc::channel();

                let reply: Option<Reply> = match msg.parse::<Command>() {
                    Ok(command) => {
                        if let Err(e) = spreadsheet.send(command, tx) {
                            Some(Reply::Error(format!(
                                "Failed to send command to spreadsheet: {}",
                                e
                            )))
                        } else {
                            let response = rx.recv().unwrap_or(Some(Reply::Error(
                                "No response from spreadsheet".to_string(),
                            )));

                            response
                        }
                    }
                    Err(e) => Some(Reply::Error(e)),
                };

                if let Some(reply) = reply {
                    match send.lock().unwrap().write_message(reply) {
                        WriteMessageResult::Ok => {}
                        WriteMessageResult::ConnectionClosed => {
                            break;
                        }
                        WriteMessageResult::Err(_) => {
                            return;
                        }
                    }
                }
            }
            ReadMessageResult::ConnectionClosed => {
                break;
            }
            ReadMessageResult::Err(_) => {
                return;
            }
        }
    }
}
