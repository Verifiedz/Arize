//! Just enough CSV for `records import` and `records export` (ADR 0024 §4): RFC 4180 fields,
//! quoted when they hold a comma, a quote or a line break, `""` for a quote inside quotes.

/// Every row of `text`, each a list of fields. A UTF-8 byte-order mark is ignored, `\r\n` and `\n`
/// both end a row, and blank lines are skipped.
pub fn parse(text: &str) -> Result<Vec<Vec<String>>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (mut rows, mut row, mut field) = (Vec::new(), Vec::new(), String::new());
    let mut chars = text.chars().peekable();
    let mut line = 1;
    let mut quoted = false;
    let end_row = |row: &mut Vec<String>, field: &mut String, rows: &mut Vec<Vec<String>>| {
        row.push(std::mem::take(field));
        let done = std::mem::take(row);
        if !(done.len() == 1 && done[0].is_empty()) {
            rows.push(done);
        }
    };
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            (true, '"') => {
                quoted = false;
                if !matches!(chars.peek(), None | Some(',' | '\r' | '\n')) {
                    return Err(format!("line {line}: text after a closing quote"));
                }
            }
            (true, c) => {
                if c == '\n' {
                    line += 1;
                }
                field.push(c);
            }
            (false, '"') if field.is_empty() => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut field)),
            (false, '\r') if chars.peek() == Some(&'\n') => {}
            (false, '\n') => {
                end_row(&mut row, &mut field, &mut rows);
                line += 1;
            }
            (false, c) => field.push(c),
        }
    }
    if quoted {
        return Err(format!("line {line}: a quote is never closed"));
    }
    end_row(&mut row, &mut field, &mut rows);
    Ok(rows)
}

/// One row, fields quoted where they need it, ending in `\n`.
pub fn row(fields: &[String]) -> String {
    let mut out = fields
        .iter()
        .map(|f| match f.contains([',', '"', '\n', '\r']) {
            true => format!("\"{}\"", f.replace('"', "\"\"")),
            false => f.clone(),
        })
        .collect::<Vec<_>>()
        .join(",");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_commas_and_line_breaks_round_trip() {
        let fields = vec!["Acme, Inc".to_owned(), "said \"hi\"".to_owned(), "two\nlines".to_owned(), String::new()];
        let text = format!("{}{}", row(&["a".into(), "b".into(), "c".into(), "d".into()]), row(&fields));
        assert_eq!(parse(&text).unwrap(), vec![vec!["a", "b", "c", "d"], fields.iter().map(String::as_str).collect()]);
    }

    #[test]
    fn crlf_bom_and_blank_lines() {
        let rows = parse("\u{feff}id,company\r\nacme,Acme\r\n\r\nglobex,Globex").unwrap();
        assert_eq!(rows, vec![vec!["id", "company"], vec!["acme", "Acme"], vec!["globex", "Globex"]]);
    }

    #[test]
    fn broken_quotes_name_the_line() {
        assert_eq!(parse("a\n\"open").unwrap_err(), "line 2: a quote is never closed");
        assert_eq!(parse("\"a\"b").unwrap_err(), "line 1: text after a closing quote");
    }
}
