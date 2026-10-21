//! Conservative preflight policy for read-only T-SQL queries.

/// Conservative preflight read-only validator for T-SQL queries.
/// Replicates the strict rules of sql-readonly.cjs.
pub fn assert_sql_read_only(sql: &str) -> Result<(), (&'static str, &'static str)> {
    let trimmed = sql.trim();
    if trimmed.is_empty() || trimmed.len() > 16_000 {
        return Err(("SQL_READONLY", "Consulta vacía o excede 16000 caracteres"));
    }

    const FORBIDDEN: &[&str] = &[
        "UPDATE",
        "DELETE",
        "INSERT",
        "MERGE",
        "DROP",
        "ALTER",
        "CREATE",
        "TRUNCATE",
        "EXEC",
        "EXECUTE",
        "GRANT",
        "DENY",
        "REVOKE",
        "INTO",
        "OPENQUERY",
        "OPENROWSET",
        "OPENDATASOURCE",
        "NEXT",
        "UPDLOCK",
        "XLOCK",
        "TABLOCK",
        "TABLOCKX",
        "HOLDLOCK",
        "SERIALIZABLE",
        "REPEATABLEREAD",
        "BEGIN",
        "COMMIT",
        "ROLLBACK",
        "SAVE",
        "SET",
        "USE",
        "DECLARE",
        "WAITFOR",
        "DBCC",
        "BACKUP",
        "RESTORE",
        "BULK",
        "KILL",
        "SHUTDOWN",
        "RECONFIGURE",
        "GO",
    ];

    let mut tokens: Vec<String> = Vec::new();
    let mut depth: isize = 0;
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        let next = if i + 1 < chars.len() {
            Some(chars[i + 1])
        } else {
            None
        };

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        // Single-line comment: -- ...
        if c == '-' && next == Some('-') {
            i += 2;
            while i < chars.len() && chars[i] != '\r' && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // Multi-line comment: /* ... */
        if c == '/' && next == Some('*') {
            i += 2;
            let mut nesting = 1;
            while i < chars.len() && nesting > 0 {
                if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
                    nesting += 1;
                    if nesting > 100 {
                        return Err(("SQL_READONLY", "Comentario demasiado anidado"));
                    }
                    i += 2;
                } else if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                    nesting -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if nesting > 0 {
                return Err(("SQL_READONLY", "Comentario multilínea sin cerrar"));
            }
            continue;
        }

        // Quoted string or identifier: '...', "...", [...]
        if c == '\'' || c == '"' || c == '[' {
            let end_char = if c == '[' { ']' } else { c };
            let mut closed = false;
            i += 1;
            while i < chars.len() {
                if chars[i] == end_char {
                    if i + 1 < chars.len() && chars[i + 1] == end_char {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(("SQL_READONLY", "Identificador o literal sin cerrar"));
            }
            tokens.push("<quoted>".to_string());
            continue;
        }

        // Word tokens
        if c.is_ascii_alphabetic() || c == '_' || c == '@' || c == '#' {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '_'
                    || chars[i] == '@'
                    || chars[i] == '$'
                    || chars[i] == '#')
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let upper = word.to_ascii_uppercase();
            if FORBIDDEN.contains(&upper.as_str()) {
                return Err((
                    "SQL_READONLY",
                    "SQL_READONLY: solo se permite un SELECT sin operaciones de escritura ni bloqueos exclusivos",
                ));
            }
            tokens.push(upper);
            continue;
        }

        if c == '(' {
            depth += 1;
            if depth > 100 {
                return Err(("SQL_READONLY", "Paréntesis demasiado anidados"));
            }
        } else if c == ')' {
            depth -= 1;
            if depth < 0 {
                return Err(("SQL_READONLY", "Paréntesis desbalanceados"));
            }
        }

        tokens.push(c.to_string());
        i += 1;
    }

    if depth != 0 {
        return Err(("SQL_READONLY", "Paréntesis desbalanceados"));
    }

    // Optional leading / trailing semicolon
    if tokens.first().map(|s| s.as_str()) == Some(";") {
        tokens.remove(0);
    }
    if tokens.last().map(|s| s.as_str()) == Some(";") {
        tokens.pop();
    }

    let first = tokens.first().map(|s| s.as_str());
    if first != Some("SELECT") && first != Some("WITH") {
        return Err((
            "SQL_READONLY",
            "La consulta debe comenzar con SELECT o WITH",
        ));
    }

    if tokens.iter().any(|s| s == ";") {
        return Err((
            "SQL_READONLY",
            "Múltiples declaraciones no permitidas en consulta SQL",
        ));
    }

    Ok(())
}
