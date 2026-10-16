use toml_edit::{DocumentMut, Item, Key, Table};

pub(super) fn read(content: &str, table: &str, key: &str) -> String {
    let Ok(document) = content.parse::<DocumentMut>() else {
        return String::new();
    };
    let mut current = document.as_item();
    if !table.is_empty() {
        let Ok(keys) = Key::parse(table) else {
            return String::new();
        };
        for key in keys {
            let Some(next) = current.get(key.get()) else {
                return String::new();
            };
            current = next;
        }
    }
    current
        .get(key)
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string().trim().to_string())
        })
        .unwrap_or_default()
}

pub(super) fn write(content: &str, table: &str, key: &str, value: Option<Item>) -> String {
    let Ok(mut document) = content.parse::<DocumentMut>() else {
        return content.to_string();
    };
    let mut current = document.as_table_mut();
    if !table.is_empty() {
        let Ok(keys) = Key::parse(table) else {
            return content.to_string();
        };
        for key in keys {
            let entry = current.entry_format(&key).or_insert_with(|| {
                let mut table = Table::new();
                table.set_implicit(true);
                Item::Table(table)
            });
            let Some(table) = entry.as_table_mut() else {
                return content.to_string();
            };
            current = table;
        }
        current.set_implicit(false);
    }
    match value {
        Some(mut value) => {
            if let (Some(old), Some(new)) = (
                current.get(key).and_then(Item::as_value),
                value.as_value_mut(),
            ) {
                *new.decor_mut() = old.decor().clone();
            }
            current.insert(key, value);
        }
        None => {
            current.remove(key);
        }
    }
    let output = document.to_string();
    output.strip_suffix('\n').unwrap_or(&output).to_string()
}

pub(super) fn raw(value: &str) -> Option<Item> {
    value.parse::<toml_edit::Value>().ok().map(Item::Value)
}
