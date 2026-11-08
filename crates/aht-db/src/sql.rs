//! SQL text built by Handrail itself (never from user text): quoted identifiers.

use crate::DbKind;

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

/// The statement that shows a table's rows.
pub fn select_rows(kind: DbKind, namespace: &str, table: &str, limit: usize) -> String {
    format!(
        "SELECT * FROM {} LIMIT {limit}",
        qualified(kind, namespace, table)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_quoted_per_dialect() {
        assert_eq!(quote_ident(DbKind::Mysql, "a`b"), "`a``b`");
        assert_eq!(quote_ident(DbKind::Postgres, "a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_ident(DbKind::Postgres, "a`b"), "\"a`b\"");
        assert_eq!(
            select_rows(DbKind::Mysql, "shop", "order items", 200),
            "SELECT * FROM `shop`.`order items` LIMIT 200"
        );
        assert_eq!(
            qualified(DbKind::Postgres, "public", "Users"),
            "\"public\".\"Users\""
        );
    }
}
