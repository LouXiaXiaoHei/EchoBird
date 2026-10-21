use toml_edit::{DocumentMut, InlineTable, Item, Key, Table};

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
    let mut current = document.as_item_mut();
    if !table.is_empty() {
        let Ok(keys) = Key::parse(table) else {
            return content.to_string();
        };
        for key in keys {
            let inline = current.is_inline_table();
            let Some(parent) = current.as_table_like_mut() else {
                return content.to_string();
            };
            current = parent.entry_format(&key).or_insert_with(|| {
                if inline {
                    return Item::Value(InlineTable::new().into());
                }
                let mut table = Table::new();
                table.set_implicit(true);
                Item::Table(table)
            });
        }
        if let Some(table) = current.as_table_mut() {
            table.set_implicit(false);
        }
    }
    let Some(current) = current.as_table_like_mut() else {
        return content.to_string();
    };
    match value {
        Some(mut value) => {
            if let Some(existing) = current.get_mut(key) {
                let value_decor = existing.as_value().map(|old| old.decor().clone());
                if let (Some(value_decor), Some(new)) = (value_decor, value.as_value_mut()) {
                    *new.decor_mut() = value_decor;
                }
                *existing = value;
            } else {
                current.insert(key, value);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_provider_tables_update_and_insert_without_changing_other_settings() {
        for config in [
            "model_providers = { OpenAI = { base_url = 'old', keep = true } } # provider comment\n",
            "model_providers = { Other = { base_url = 'other', keep = true } } # provider comment\n",
        ] {
            let updated = write(
                config,
                "model_providers.OpenAI",
                "base_url",
                Some(toml_edit::value("new")),
            );
            assert_eq!(read(&updated, "model_providers.OpenAI", "base_url"), "new");
            assert!(updated.contains("keep = true"));
            assert!(updated.contains("# provider comment"));
            assert!(updated.parse::<DocumentMut>().is_ok());
            if config.contains("Other") {
                assert_eq!(read(&updated, "model_providers.Other", "base_url"), "other");
            }
            assert_eq!(
                write(&updated, "model_providers.OpenAI", "base_url", Some(toml_edit::value("new"))),
                updated
            );
        }
    }
}
