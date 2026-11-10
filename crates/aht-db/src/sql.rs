//! SQL text built by Handrail itself (never from user text): quoted identifiers, and the
//! statements that apply edits made in the result grid.

use crate::{ColumnInfo, DbKind};
use aht_core::{Error, Result};
use serde::Deserialize;

/// `name` as a quoted identifier: `` `a``b` `` on MySQL, `"a""b"` on PostgreSQL.
pub fn quote_ident(kind: DbKind, name: &str) -> String {
    let q = match kind {
        DbKind::Mysql => '`',
        DbKind::Postgres => '"',
    };
    let mut out = String::with_capacity(name.len() + 2);
    out.push(q);
    for c in name.chars() {
        if c == q {
            out.push(q);
        }
        out.push(c);
    }
    out.push(q);
    out
}

/// `namespace.table`, both quoted.
pub fn qualified(kind: DbKind, namespace: &str, table: &str) -> String {
    format!(
        "{}.{}",
        quote_ident(kind, namespace),
        quote_ident(kind, table)
    )
}

/// Which of a table's rows to show.
#[derive(Clone, Debug, Default)]
pub struct RowsQuery<'a> {
    pub limit: usize,
    pub offset: usize,
    /// A column to sort by, descending when `true`.
    pub order: Option<(&'a str, bool)>,
    /// A `WHERE` condition the user typed (SQL, as in the editor).
    pub filter: Option<&'a str>,
}

/// The statement that shows a table's rows. The filter goes on lines of its own, so a `--`
/// comment in it cannot swallow what follows.
pub fn select_rows(kind: DbKind, namespace: &str, table: &str, q: &RowsQuery) -> String {
    let mut sql = format!("SELECT * FROM {}", qualified(kind, namespace, table));
    if let Some(f) = q.filter.map(str::trim).filter(|f| !f.is_empty()) {
        sql.push_str(&format!("\nWHERE (\n{f}\n)"));
    }
    if let Some((col, desc)) = q.order {
        sql.push_str(&format!(
            "\nORDER BY {} {}",
            quote_ident(kind, col),
            if desc { "DESC" } else { "ASC" }
        ));
    }
    sql.push_str(&format!("\nLIMIT {}", q.limit));
    if q.offset > 0 {
        sql.push_str(&format!(" OFFSET {}", q.offset));
    }
    sql
}

/// A cell value sent by the grid: `None` is SQL NULL, text is converted by the server to the
/// column's type.
pub type CellValue = Option<String>;

/// One change to one row of a table, made in the grid. Rows are found by their primary key.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RowEdit {
    Update {
        /// The primary key's columns and their values before the edit.
        key: Vec<(String, CellValue)>,
        set: Vec<(String, CellValue)>,
    },
    Insert {
        /// Columns left out take their default.
        values: Vec<(String, CellValue)>,
    },
    Delete {
        key: Vec<(String, CellValue)>,
    },
}

/// A statement with its parameters, in placeholder order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditStatement {
    pub sql: String,
    pub params: Vec<CellValue>,
    /// It must change at most one row (an update or delete by primary key).
    pub single_row: bool,
}

/// The statements for `edits` on `namespace.table`, whose columns are `columns`: deletes,
/// then updates, then inserts. Fails for a table without a primary key, a key that is not
/// exactly the primary key, or a column the table does not have.
pub fn edit_statements(
    kind: DbKind,
    namespace: &str,
    table: &str,
    columns: &[ColumnInfo],
    edits: &[RowEdit],
) -> Result<Vec<EditStatement>> {
    let target = qualified(kind, namespace, table);
    let pk: Vec<&str> = columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect();
    let column = |name: &str| {
        columns
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| Error::Invalid(format!("{table} has no column {name}")))
    };
    let check_key = |key: &[(String, CellValue)]| -> Result<()> {
        if pk.is_empty() {
            return Err(Error::Invalid(format!(
                "{table} has no primary key: its rows cannot be edited here"
            )));
        }
        let mut names: Vec<&str> = key.iter().map(|(n, _)| n.as_str()).collect();
        names.sort_unstable();
        let mut want = pk.clone();
        want.sort_unstable();
        if names != want {
            return Err(Error::Invalid(format!(
                "a row of {table} is identified by its primary key ({})",
                pk.join(", ")
            )));
        }
        Ok(())
    };
    // `$n` (PostgreSQL, cast from text to the column's type) or `?` (MySQL converts itself).
    let placeholder = |n: usize, col: &ColumnInfo| match kind {
        DbKind::Mysql => "?".to_string(),
        DbKind::Postgres => format!("CAST(${n}::text AS {})", col.data_type),
    };
    let condition = |key: &[(String, CellValue)], params: &mut Vec<CellValue>| -> Result<String> {
        let mut parts = Vec::new();
        for (name, value) in key {
            let col = column(name)?;
            match value {
                None => parts.push(format!("{} IS NULL", quote_ident(kind, name))),
                Some(_) => {
                    params.push(value.clone());
                    parts.push(format!(
                        "{} = {}",
                        quote_ident(kind, name),
                        placeholder(params.len(), col)
                    ));
                }
            }
        }
        Ok(parts.join(" AND "))
    };

    let mut out = Vec::new();
    for e in edits {
        if let RowEdit::Delete { key } = e {
            check_key(key)?;
            let mut params = Vec::new();
            let cond = condition(key, &mut params)?;
            out.push(EditStatement {
                sql: format!("DELETE FROM {target} WHERE {cond}"),
                params,
                single_row: true,
            });
        }
    }
    for e in edits {
        if let RowEdit::Update { key, set } = e {
            check_key(key)?;
            if set.is_empty() {
                continue;
            }
            let mut params = Vec::new();
            let mut assignments = Vec::new();
            for (name, value) in set {
                let col = column(name)?;
                params.push(value.clone());
                assignments.push(format!(
                    "{} = {}",
                    quote_ident(kind, name),
                    placeholder(params.len(), col)
                ));
            }
            let cond = condition(key, &mut params)?;
            out.push(EditStatement {
                sql: format!(
                    "UPDATE {target} SET {} WHERE {cond}",
                    assignments.join(", ")
                ),
                params,
                single_row: true,
            });
        }
    }
    for e in edits {
        if let RowEdit::Insert { values } = e {
            let mut params = Vec::new();
            let mut names = Vec::new();
            let mut holes = Vec::new();
            for (name, value) in values {
                let col = column(name)?;
                params.push(value.clone());
                names.push(quote_ident(kind, name));
                holes.push(placeholder(params.len(), col));
            }
            let sql = if names.is_empty() {
                match kind {
                    DbKind::Mysql => format!("INSERT INTO {target} () VALUES ()"),
                    DbKind::Postgres => format!("INSERT INTO {target} DEFAULT VALUES"),
                }
            } else {
                format!(
                    "INSERT INTO {target} ({}) VALUES ({})",
                    names.join(", "),
                    holes.join(", ")
                )
            };
            out.push(EditStatement {
                sql,
                params,
                single_row: false,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_quoted_per_dialect() {
        assert_eq!(quote_ident(DbKind::Mysql, "a`b"), "`a``b`");
        assert_eq!(quote_ident(DbKind::Postgres, "a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_ident(DbKind::Postgres, "a`b"), "\"a`b\"");
        let page = RowsQuery {
            limit: 200,
            ..Default::default()
        };
        assert_eq!(
            select_rows(DbKind::Mysql, "shop", "order items", &page),
            "SELECT * FROM `shop`.`order items`\nLIMIT 200"
        );
        let q = RowsQuery {
            limit: 50,
            offset: 100,
            order: Some(("na\"me", true)),
            filter: Some(" qty > 3 -- big "),
        };
        assert_eq!(
            select_rows(DbKind::Postgres, "public", "t", &q),
            "SELECT * FROM \"public\".\"t\"\nWHERE (\nqty > 3 -- big\n)\n\
             ORDER BY \"na\"\"me\" DESC\nLIMIT 50 OFFSET 100"
        );
        assert_eq!(
            qualified(DbKind::Postgres, "public", "Users"),
            "\"public\".\"Users\""
        );
    }

    fn cols() -> Vec<ColumnInfo> {
        let c = |name: &str, data_type: &str, pk: bool| ColumnInfo {
            name: name.into(),
            data_type: data_type.into(),
            nullable: !pk,
            default: None,
            primary_key: pk,
        };
        vec![
            c("id", "integer", true),
            c("name", "character varying(40)", false),
            c("note", "text", false),
        ]
    }

    fn key(id: &str) -> Vec<(String, CellValue)> {
        vec![("id".into(), Some(id.into()))]
    }

    #[test]
    fn edits_become_parameterised_statements_in_a_safe_order() {
        let edits = vec![
            RowEdit::Insert {
                values: vec![("name".into(), Some("ada".into()))],
            },
            RowEdit::Update {
                key: key("2"),
                set: vec![
                    ("name".into(), Some("O'Brien".into())),
                    ("note".into(), None),
                ],
            },
            RowEdit::Delete { key: key("3") },
        ];
        let my = edit_statements(DbKind::Mysql, "shop", "people", &cols(), &edits).unwrap();
        assert_eq!(
            my.iter().map(|s| s.sql.as_str()).collect::<Vec<_>>(),
            [
                "DELETE FROM `shop`.`people` WHERE `id` = ?",
                "UPDATE `shop`.`people` SET `name` = ?, `note` = ? WHERE `id` = ?",
                "INSERT INTO `shop`.`people` (`name`) VALUES (?)",
            ]
        );
        assert_eq!(
            my[1].params,
            [Some("O'Brien".to_string()), None, Some("2".to_string())]
        );
        assert!(my[0].single_row && my[1].single_row && !my[2].single_row);

        let pg = edit_statements(DbKind::Postgres, "public", "people", &cols(), &edits).unwrap();
        assert_eq!(
            pg[1].sql,
            "UPDATE \"public\".\"people\" SET \"name\" = CAST($1::text AS character varying(40)), \
             \"note\" = CAST($2::text AS text) WHERE \"id\" = CAST($3::text AS integer)"
        );
    }

    #[test]
    fn edits_need_the_primary_key_and_real_columns() {
        let mut no_pk = cols();
        no_pk[0].primary_key = false;
        let del = [RowEdit::Delete { key: key("1") }];
        assert!(edit_statements(DbKind::Mysql, "s", "t", &no_pk, &del).is_err());
        let by_name = [RowEdit::Delete {
            key: vec![("name".into(), Some("x".into()))],
        }];
        assert!(edit_statements(DbKind::Mysql, "s", "t", &cols(), &by_name).is_err());
        let bad_column = [RowEdit::Update {
            key: key("1"),
            set: vec![("nope; DROP TABLE t".into(), None)],
        }];
        assert!(edit_statements(DbKind::Mysql, "s", "t", &cols(), &bad_column).is_err());
        let empty = [RowEdit::Insert { values: vec![] }];
        assert_eq!(
            edit_statements(DbKind::Postgres, "s", "t", &cols(), &empty).unwrap()[0].sql,
            "INSERT INTO \"s\".\"t\" DEFAULT VALUES"
        );
    }
}
