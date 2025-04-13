use std::{
    collections::HashMap,
    sync::mpsc::{self, SendError, Sender},
    thread::{self},
};

use regex::Regex;
use rsheet_lib::{
    cell_expr::{CellArgument, CellExpr, CellExprEvalError},
    cell_value::CellValue,
    cells::{column_name_to_number, column_number_to_name},
    command::{CellIdentifier, Command},
    replies::Reply,
};

#[derive(Default)]
struct Spreadsheet {
    spreadsheet: HashMap<CellIdentifier, CellValue>,
}

/// Error message used for when a Cell relies on another cell which
/// results in an Error
const DEPENDANCY_ERROR: &str = "Cell relies on an Error Value";

impl Spreadsheet {
    fn new() -> Self {
        Self {
            spreadsheet: HashMap::new(),
        }
    }

    fn insert(&mut self, cell_identifier: &CellIdentifier, cell_expr: &str) {
        let cell_expr = CellExpr::new(cell_expr);
        let arguments = cell_expr.find_variable_names();
        let arg_map = create_arg_map(arguments, self);
        let result = cell_expr.evaluate(&arg_map);

        match result {
            Ok(cell_value) => {
                self.spreadsheet.insert(*cell_identifier, cell_value);
            }
            Err(CellExprEvalError::VariableDependsOnError) => {
                self.spreadsheet.insert(
                    *cell_identifier,
                    CellValue::Error(DEPENDANCY_ERROR.to_string()),
                );
            }
        }
    }

    fn find(&self, cell_identifier: &CellIdentifier) -> Result<CellValue, CellExprEvalError> {
        let value = self
            .spreadsheet
            .get(cell_identifier)
            .unwrap_or(&CellValue::None)
            .clone();

        match value {
            CellValue::Error(e) if e == DEPENDANCY_ERROR.to_string() => {
                Err(CellExprEvalError::VariableDependsOnError)
            }
            _ => Ok(value),
        }
    }
}

pub struct SpreadSheetHandler {
    command_tx: Sender<(Command, Sender<Option<Reply>>)>,
}

impl SpreadSheetHandler {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<(Command, Sender<Option<Reply>>)>();
        thread::spawn(move || {
            let mut spreadsheet = Spreadsheet::new();

            for (command, responder) in rx {
                match command {
                    Command::Get { cell_identifier } => {
                        let value = spreadsheet.find(&cell_identifier);
                        match value {
                            Ok(v) => responder
                                .send(Some(Reply::Value(format_cell_name(&cell_identifier), v)))
                                .unwrap(),
                            Err(e) => match e {
                                CellExprEvalError::VariableDependsOnError => {
                                    responder
                                        .send(Some(Reply::Error(
                                            "Cell depends on an Error Cell".to_string(),
                                        )))
                                        .unwrap();
                                }
                            },
                        }
                    }
                    Command::Set {
                        cell_identifier,
                        cell_expr,
                    } => {
                        spreadsheet.insert(&cell_identifier, &cell_expr);
                        let _ = responder.send(None);
                    }
                }
            }
        });

        SpreadSheetHandler { command_tx: tx }
    }

    pub fn send(
        &self,
        cmd: Command,
        sender: Sender<Option<Reply>>,
    ) -> Result<(), SendError<(Command, Sender<Option<Reply>>)>> {
        self.command_tx.send((cmd, sender))
    }
}

fn format_cell_name(cell: &CellIdentifier) -> String {
    format!("{}{}", column_number_to_name(cell.col), cell.row + 1)
}

fn create_arg_map(args: Vec<String>, spreadsheet: &Spreadsheet) -> HashMap<String, CellArgument> {
    args.iter()
        .map(|a| (a.clone(), string_arg_to_cell_arg(a.clone(), spreadsheet)))
        .collect::<HashMap<String, CellArgument>>()
}

fn string_arg_to_cell_arg(arg: String, spreadsheet: &Spreadsheet) -> CellArgument {
    if arg.contains("_") {
        let re = Regex::new(r"([A-Z]+)([1-9][0-9]*)_([A-Z]+)([1-9][0-9]*)").unwrap();
        if let Some(captures) = re.captures(arg.as_str()) {
            let first_col = &captures[1];
            let first_row = &captures[2];
            let second_col = &captures[3];
            let second_row = &captures[4];
            if first_col == second_col || first_row == second_row {
                return create_cell_arg_vec(
                    first_col,
                    first_row,
                    second_col,
                    second_row,
                    spreadsheet,
                );
            } else {
                return create_cell_arg_matrix(
                    first_col,
                    first_row,
                    second_col,
                    second_row,
                    spreadsheet,
                );
            }
        } else {
            // This should be caught by the parser
            panic!("We shouldnt be here, Invalid cell when building args")
        }
    } else {
        let cell_id: CellIdentifier = arg.parse().unwrap();
        let value = match spreadsheet.find(&cell_id) {
            Ok(cell_value) => cell_value,
            Err(_) => CellValue::Error("invalid cell".to_string()),
        };
        return CellArgument::Value(value);
    }
}

fn create_cell_arg_vec(
    first_col: &str,
    first_row: &str,
    second_col: &str,
    second_row: &str,
    spreadsheet: &Spreadsheet,
) -> CellArgument {
    let mut result: Vec<CellValue> = Vec::new();

    let start_col = column_name_to_number(first_col);
    let finish_col = column_name_to_number(second_col);
    let start_row = first_row.parse::<u32>().unwrap();
    let finish_row = second_row.parse::<u32>().unwrap();

    for col in start_col..=finish_col {
        for row in start_row..=finish_row {
            let row = row - 1;
            let cell_id = CellIdentifier { col, row };
            let cell_value = match spreadsheet.find(&cell_id) {
                Ok(cell_value) => cell_value,
                Err(_) => CellValue::Error("CellExprEvalError".to_string()),
            };
            result.push(cell_value);
        }
    }

    CellArgument::Vector(result)
}

fn create_cell_arg_matrix(
    first_col: &str,
    first_row: &str,
    second_col: &str,
    second_row: &str,
    spreadsheet: &Spreadsheet,
) -> CellArgument {
    let mut result: Vec<Vec<CellValue>> = Vec::new();

    let start_col = column_name_to_number(first_col);
    let finish_col = column_name_to_number(second_col);
    let start_row = first_row.parse::<u32>().unwrap();
    let finish_row = second_row.parse::<u32>().unwrap();

    for row in start_row..=finish_row {
        let mut inner_res: Vec<CellValue> = Vec::new();
        let row = row - 1;
        for col in start_col..=finish_col {
            let cell_id = CellIdentifier { col, row };
            let cell_value = match spreadsheet.find(&cell_id) {
                Ok(cell_value) => cell_value,
                Err(_) => CellValue::Error("CellExprEvalError".to_string()),
            };
            inner_res.push(cell_value);
        }
        result.push(inner_res);
    }

    CellArgument::Matrix(result)
}
