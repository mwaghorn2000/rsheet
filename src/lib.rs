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
use std::sync::Arc;

use log::info;

pub fn start_server<M>(mut manager: M) -> Result<(), Box<dyn Error>>
where
    M: Manager,
{
    let spreadsheet = Arc::new(SpreadSheetHandler::new());

    loop {
        match manager.accept_new_connection() {
            Connection::NewConnection { reader, writer } => {
                let spreadsheet_clone = Arc::clone(&spreadsheet);
                std::thread::spawn(move || {
                    handle_connection(reader, writer, spreadsheet_clone)
                });
            },
            Connection::NoMoreConnections => {
                return Ok(());
            }
        };
    }
}

fn handle_connection<R, W>(mut recv: R, mut send: W, spreadsheet: Arc<SpreadSheetHandler>)
where
    R: Reader,
    W: Writer,
{
    loop {
        info!("Just got message");
        match recv.read_message() {
            ReadMessageResult::Message(msg) => {
                let reply: Option<Reply> = match msg.parse::<Command>() {
                    Ok(command) => spreadsheet.send(command),
                    Err(e) => Some(Reply::Error(e)),
                };

                // Skip none replies from set commands. Internal state is changed, but
                // we dont need to report this until a get is called.
                if reply.is_none() {
                    continue;
                }

                match send.write_message(reply.unwrap()) {
                    WriteMessageResult::Ok => {
                        // Message successfully sent, continue.
                    }
                    WriteMessageResult::ConnectionClosed => {
                        // The connection was closed. This is not an error, but
                        // should terminate this connection.
                        break;
                    }
                    WriteMessageResult::Err(_) => {
                        // An unexpected error was encountered.
                        // Exit the thread
                        return;
                    }
                }
            }
            ReadMessageResult::ConnectionClosed => {
                // The connection was closed. This is not an error, but
                // should terminate this connection.
                break;
            }
            ReadMessageResult::Err(_) => {
                // An unexpected error was encountered.
                // Exit the thread
                return;
            }
        }
    }
}
