//! Jinja filters mirroring the Django filters used by the templates.

use minijinja::{Error, Value};

fn truthy(value: &Value) -> bool {
    value.is_true()
}

fn as_text(value: &Value) -> String {
    match value.as_str() {
        Some(text) => text.to_string(),
        None => value.to_string(),
    }
}

fn as_number(value: &Value) -> Option<f64> {
    if let Some(number) = value.as_i64() {
        return Some(number as f64);
    }
    if let Some(text) = value.as_str() {
        return text.parse().ok();
    }
    serde_json::to_value(value).ok().and_then(|v| v.as_f64())
}

/// Django's `default`: use the fallback for any falsy value.
pub fn ddefault(value: Value, arg: Value) -> Value {
    if truthy(&value) {
        value
    } else {
        arg
    }
}

/// Django's `default_if_none`.
pub fn ddefault_if_none(value: Value, arg: Value) -> Value {
    if value.is_none() || value.is_undefined() {
        arg
    } else {
        value
    }
}

pub fn truncatechars(value: Value, count: Value) -> Value {
    let text = as_text(&value);
    let limit = count.as_i64().unwrap_or(0).max(0) as usize;
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= limit {
        Value::from(text)
    } else {
        let truncated: String = chars[..limit].iter().collect();
        Value::from(format!("{truncated}…"))
    }
}

pub fn truncatewords(value: Value, count: Value) -> Value {
    let text = as_text(&value);
    let limit = count.as_i64().unwrap_or(0).max(0) as usize;
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= limit {
        Value::from(text)
    } else {
        Value::from(format!("{} …", words[..limit].join(" ")))
    }
}

pub fn floatformat(value: Value, arg: Option<Value>) -> Value {
    let Some(number) = as_number(&value) else {
        return Value::from("");
    };
    let decimals = match arg {
        None => 1,
        Some(arg) => arg
            .as_i64()
            .or_else(|| arg.as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(1)
            .max(0) as usize,
    };
    Value::from(format!("{number:.decimals$}"))
}

pub fn yesno(value: Value, arg: Value) -> Value {
    let choices: Vec<String> = arg
        .as_str()
        .unwrap_or("yes,no,maybe")
        .split(',')
        .map(str::to_string)
        .collect();
    let text = if truthy(&value) {
        choices.first().cloned().unwrap_or_default()
    } else {
        choices.get(1).cloned().unwrap_or_default()
    };
    Value::from(text)
}

pub fn pluralize(value: Value, arg: Option<Value>) -> Value {
    let number = as_number(&value).unwrap_or(0.0);
    let choices: Vec<String> = arg
        .as_ref()
        .and_then(Value::as_str)
        .unwrap_or("")
        .split(',')
        .map(str::to_string)
        .filter(|part| !part.is_empty())
        .collect();
    let text = if choices.len() < 2 {
        if number == 1.0 { "" } else { "s" }.to_string()
    } else if number == 1.0 {
        choices[0].clone()
    } else {
        choices[1].clone()
    };
    Value::from(text)
}

/// Django's `escapejs`, including the same escape table.
pub fn escapejs(value: Value) -> Result<String, Error> {
    let text = as_text(&value);
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\u0022"),
            '\'' => out.push_str("\\u0027"),
            '>' => out.push_str("\\u003E"),
            '<' => out.push_str("\\u003C"),
            '&' => out.push_str("\\u0026"),
            '=' => out.push_str("\\u003D"),
            '-' => out.push_str("\\u002D"),
            ';' => out.push_str("\\u003B"),
            '`' => out.push_str("\\u0060"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    Ok(out)
}

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September",
    "October", "November", "December",
];

/// Subset of Django's `date` filter format codes used by the templates.
pub fn ddate(value: Value, arg: Value) -> Value {
    let text = as_text(&value);
    let Some(format) = arg.as_str() else {
        return Value::from(text.chars().take(10).collect::<String>());
    };
    let bytes: Vec<&str> = text.split(['-', 'T', ' ']).collect();
    if bytes.len() < 3 {
        return Value::from(text);
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        bytes[0].parse::<i32>(),
        bytes[1].parse::<u32>(),
        bytes[2].parse::<u32>(),
    ) else {
        return Value::from(text);
    };
    if !(1..=12).contains(&month) {
        return Value::from(text);
    }
    let mut out = String::new();
    for ch in format.chars() {
        match ch {
            'Y' => out.push_str(&format!("{year:04}")),
            'y' => out.push_str(&format!("{:02}", year.rem_euclid(100))),
            'm' => out.push_str(&format!("{month:02}")),
            'n' => out.push_str(&month.to_string()),
            'd' => out.push_str(&format!("{day:02}")),
            'j' => out.push_str(&day.to_string()),
            'F' => out.push_str(MONTHS[(month - 1) as usize]),
            'M' => out.push_str(&MONTHS[(month - 1) as usize][..3]),
            other => out.push(other),
        }
    }
    Value::from(out)
}
