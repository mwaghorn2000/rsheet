use std::{
    collections::{HashMap, HashSet},
    sync::{
        mpsc::{self, Sender}, Arc, Mutex, RwLock
    }, thread,
};

use regex::Regex;
use rsheet_lib::{
    cell_expr::{CellArgument, CellExpr, CellExprEvalError},
    cell_value::CellValue,
    cells::{column_name_to_number, column_number_to_name},
    command::{CellIdentifier, Command},
    replies::Reply,
};

type DependancySender = Option<Sender<(CellIdentifier, u64)>>;

/// Error message used for when a Cell relies on another cell which
/// results in an Error
const DEPENDANCY_ERROR: &str = "Cell relies on an Error Value";

pub struct SpreadSheetHandler {
    spread_sheet: Arc<RwLock<HashMap<CellIdentifier, CellValue>>>,
    dependancy_graph: Arc<RwLock<HashMap<CellIdentifier, HashSet<CellIdentifier>>>>,
    cell_exprs: Arc<RwLock<HashMap<CellIdentifier, String>>>,
    dependancy_sender: Arc<Mutex<DependancySender>>,
    version_map: Arc<RwLock<HashMap<CellIdentifier, u64>>>,
}

impl SpreadSheetHandler {
    pub fn new() -> Self {
        SpreadSheetHandler {
            spread_sheet: Arc::new(RwLock::new(HashMap::new())),
            dependancy_graph: Arc::new(RwLock::new(HashMap::new())),
            cell_exprs: Arc::new(RwLock::new(HashMap::new())),
            dependancy_sender: Arc::new(Mutex::new(None)),
            version_map: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Starts a dependancy thread which will handle all dependancies through the 
    /// dependancy_sender. Only one thread may be spawned.
    pub fn start_dependancy_thread(this: Arc<Self>) {
        let thread_exists = {
            let guard = this.dependancy_sender.lock().unwrap();
            guard.is_some()
        };

        if thread_exists { return; }

        let (tx, rx) = mpsc::channel::<(CellIdentifier, u64)>();
        {
            let mut sender = this.dependancy_sender.lock().unwrap();
            *sender = Some(tx);
        }
        
        let spread_sheet_clone = Arc::clone(&this);

        thread::spawn(move || {
            for (cell, version) in rx {
                let expr = {
                    let cell_expr = spread_sheet_clone.cell_exprs.read().unwrap();
                    cell_expr.get(&cell).unwrap().clone()
                };
        
                let value = spread_sheet_clone.evaluate(expr, &cell);

                let versions = this.version_map.read().unwrap();
                let curr_version = *versions.get(&cell).unwrap_or(&0);
                if curr_version == version {
                    this.insert(&cell, value);
                }
            }
        });
    }

    pub fn send(&self, cmd: Command) -> Option<Reply> {
        match cmd {
            Command::Get { cell_identifier } => {
                let value = self.find(&cell_identifier);
                match value {
                    Ok(v) => Some(Reply::Value(format_cell_name(&cell_identifier), v)),
                    Err(e) => match e {
                        CellExprEvalError::VariableDependsOnError => {
                            Some(Reply::Error("Cell depends on an Error Cell".to_string()))
                        }
                    },
                }
            }
            Command::Set {
                cell_identifier,
                cell_expr,
            } => {
                let expected_version = {
                    let mut versions = self.version_map.write().unwrap();
                    let v = versions.entry(cell_identifier.clone()).or_insert(0);
                    *v += 1;
                    *v
                };

                let cell_value = self.evaluate(cell_expr.clone(), &cell_identifier);

                let versions = self.version_map.read().unwrap();
                let curr_version = *versions.get(&cell_identifier).unwrap_or(&0);
                if curr_version == expected_version {
                    {
                        let mut exprs = self.cell_exprs.write().unwrap();
                        exprs.insert(cell_identifier, cell_expr);
                    }
                    self.insert(&cell_identifier, cell_value);
                }

                None
            }
        }
    }

    fn insert(&self, cell_identifier: &CellIdentifier, cell_value: Result<CellValue, CellExprEvalError>) {
        match cell_value {
            Ok(v) => {
                let mut sheet = self.spread_sheet.write().unwrap();
                sheet.insert(*cell_identifier, v);
            }
            Err(CellExprEvalError::VariableDependsOnError) => {
                let mut sheet = self.spread_sheet.write().unwrap();
                sheet.insert(
                    *cell_identifier,
                    CellValue::Error(DEPENDANCY_ERROR.to_string()),
                );
            }
        }

        if let Some(dependancies) = self.dependancy_graph.read().unwrap().get(cell_identifier) {
            for d in dependancies {
                let version = *self.version_map.read().unwrap().get(d).unwrap_or(&0);
                let sender = {
                    let guard = self.dependancy_sender.lock().unwrap();
                    guard.clone()
                };

                if let Some(sender) = sender {
                    let _ = sender.send((*d, version));
                }
            }
        }
        
    }

    fn find(&self, cell_identifier: &CellIdentifier) -> Result<CellValue, CellExprEvalError> {
        let sheet = self.spread_sheet.read().unwrap();
        let value = sheet
            .get(cell_identifier)
            .unwrap_or(&CellValue::None)
            .clone();

        match value {
            CellValue::Error(e) if e == *DEPENDANCY_ERROR => {
                Err(CellExprEvalError::VariableDependsOnError)
            }
            _ => Ok(value),
        }
    }

    fn evaluate(&self, cell_expr: String, cell: &CellIdentifier) -> Result<CellValue, CellExprEvalError> {
        let cell_expr = CellExpr::new(&cell_expr);
        let arguments = cell_expr.find_variable_names();
        let sheet = self.spread_sheet.read().unwrap();
        let arg_map = create_arg_map(arguments.clone(), &sheet);
        drop(sheet);

        {
            let mut dg = self.dependancy_graph.write().unwrap();

            for dependants in dg.values_mut() {
                dependants.remove(cell);
            }

            for arg in arguments {
                for id in extract_cell_identifiers_from_arg(&arg) {
                    dg.entry(id).or_default().insert(*cell);
                }
            }
        }

        cell_expr.evaluate(&arg_map)
    }
}

fn format_cell_name(cell: &CellIdentifier) -> String {
    format!("{}{}", column_number_to_name(cell.col), cell.row + 1)
}

fn create_arg_map(args: Vec<String>, spreadsheet: &HashMap<CellIdentifier, CellValue>) -> HashMap<String, CellArgument> {
    args.iter()
        .map(|a| (a.clone(), string_arg_to_cell_arg(a.clone(), spreadsheet)))
        .collect::<HashMap<String, CellArgument>>()
}

fn string_arg_to_cell_arg(arg: String, spreadsheet: &HashMap<CellIdentifier, CellValue>) -> CellArgument {
    if arg.contains("_") {
        let re = Regex::new(r"([A-Z]+)([1-9][0-9]*)_([A-Z]+)([1-9][0-9]*)").unwrap();
        if let Some(captures) = re.captures(arg.as_str()) {
            let first_col = &captures[1];
            let first_row = &captures[2];
            let second_col = &captures[3];
            let second_row = &captures[4];

            let matrix = create_cell_arg_matrix(
                first_col,
                first_row,
                second_col,
                second_row,
                spreadsheet,
            );

            match (matrix.len(), matrix[0].len()) {
                (1, 1) => CellArgument::Value(matrix[0][0].clone()),
                (1, _) => CellArgument::Vector(matrix[0].clone()),
                (_, 1) => CellArgument::Vector(matrix.iter().map(|row| row[0].clone()).collect()),
                _ => CellArgument::Matrix(matrix),
            }
        } else {
            // This should be caught by the parser
            panic!("We shouldnt be here, Invalid cell when building args")
        }
    } else {
        let cell_id: CellIdentifier = arg.parse().unwrap();
        let value = spreadsheet.get(&cell_id).cloned().unwrap_or(CellValue::None);
        CellArgument::Value(value)
    }
}

fn create_cell_arg_matrix(
    first_col: &str,
    first_row: &str,
    second_col: &str,
    second_row: &str,
    spreadsheet: &HashMap<CellIdentifier, CellValue>,
) -> Vec<Vec<CellValue>> {
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
            let cell_value = match spreadsheet.get(&cell_id) {
                Some(cell_value) => cell_value.clone(),
                None => CellValue::None,
            };
            inner_res.push(cell_value);
        }
        result.push(inner_res);
    }

    result
}

fn extract_cell_identifiers_from_arg(arg: &str) -> Vec<CellIdentifier> {
    if let Some(captures) = Regex::new(r"([A-Z]+)([1-9][0-9]*)_([A-Z]+)([1-9][0-9]*)")
        .unwrap()
        .captures(arg)
    {
        let first_col = &captures[1];
        let first_row = &captures[2];
        let second_col = &captures[3];
        let second_row = &captures[4];

        return extract_range(first_col, first_row, second_col, second_row);
    }

    vec![arg.parse::<CellIdentifier>().unwrap()]
}

fn extract_range(
    first_col: &str,
    first_row: &str,
    second_col: &str,
    second_row: &str,
) -> Vec<CellIdentifier> {
    let start_col = column_name_to_number(first_col);
    let end_col = column_name_to_number(second_col);
    let start_row = first_row.parse::<u32>().unwrap() - 1;
    let end_row = second_row.parse::<u32>().unwrap() - 1;

    let mut result = Vec::new();
    for row in start_row..=end_row {
        for col in start_col..=end_col {
            result.push(CellIdentifier { col, row });
        }
    }

    result
}