//! Accounts and privileges: the statements are built here, never from SQL the page sends, and
//! passwords are quoted for the server's settings (they cannot be bound parameters in
//! CREATE/ALTER USER). The page gets a preview with the password masked.

use crate::sql::quote_ident;
use crate::DbKind;
use aht_core::{Error, Result};
use serde::Deserialize;

/// A change to an account. `host` is MySQL's account host (`%` when empty); PostgreSQL roles
/// have none.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AccountChange {
    Create {
        user: String,
        #[serde(default)]
        host: Option<String>,
        password: String,
    },
    Password {
        user: String,
        #[serde(default)]
        host: Option<String>,
        password: String,
    },
    Drop {
        user: String,
        #[serde(default)]
        host: Option<String>,
    },
    /// Privileges on every table of `namespace` (a MySQL database or PostgreSQL schema), or on
    /// everything (MySQL only) when it is `None`.
    Grant {
        user: String,
        #[serde(default)]
        host: Option<String>,
        privileges: Vec<String>,
        #[serde(default)]
        namespace: Option<String>,
    },
    Revoke {
        user: String,
        #[serde(default)]
        host: Option<String>,
        privileges: Vec<String>,
        #[serde(default)]
        namespace: Option<String>,
    },
}

impl AccountChange {
    /// The password it carries, to wipe after use.
    pub fn password_mut(&mut self) -> Option<&mut String> {
        match self {
            AccountChange::Create { password, .. } | AccountChange::Password { password, .. } => {
                Some(password)
            }
            _ => None,
        }
    }
}

/// The privileges the page may ask for: anything else is refused.
pub const PRIVILEGES: &[&str] = &[
    "ALL PRIVILEGES",
    "SELECT",
    "INSERT",
    "UPDATE",
    "DELETE",
    "CREATE",
    "DROP",
    "ALTER",
    "INDEX",
    "REFERENCES",
    "TRIGGER",
    "EXECUTE",
    "CREATE VIEW",
    "SHOW VIEW",
    "TRUNCATE",
];

/// A string literal. MySQL reads backslashes as escapes unless its `NO_BACKSLASH_ESCAPES` mode
/// is on; PostgreSQL's E'' strings always do, so they are used whatever its settings.
fn literal(kind: DbKind, s: &str, backslash_escapes: bool) -> Result<String> {
    if s.contains('\0') {
        return Err(Error::Invalid(
            "a password cannot contain a NUL character".into(),
        ));
    }
    Ok(match kind {
        DbKind::Mysql if backslash_escapes => {
            format!("'{}'", s.replace('\\', "\\\\").replace('\'', "''"))
        }
        DbKind::Mysql => format!("'{}'", s.replace('\'', "''")),
        DbKind::Postgres => format!("E'{}'", s.replace('\\', "\\\\").replace('\'', "''")),
    })
}

/// The name as given: an existing account's name is used exactly as the server listed it
/// (trimming `" app"` would name another account). The dialog trims a new one.
fn name(user: &str) -> Result<&str> {
    let u = user;
    if u.trim().is_empty() || u.len() > 63 || u.contains(['\0', '\n', '\r']) {
        return Err(Error::Invalid(
            "give the account a name (one line, at most 63 characters)".into(),
        ));
    }
    Ok(u)
}

/// `'user'@'host'` on MySQL (quoted for the session's escape mode, see [`literal`]),
/// `"role"` on PostgreSQL.
pub(crate) fn account(
    kind: DbKind,
    user: &str,
    host: Option<&str>,
    backslash_escapes: bool,
) -> Result<String> {
    let user = name(user)?;
    Ok(match kind {
        DbKind::Mysql => {
            let host = host.filter(|h| !h.trim().is_empty()).unwrap_or("%");
            format!(
                "{}@{}",
                literal(kind, user, backslash_escapes)?,
                literal(kind, host, backslash_escapes)?
            )
        }
        DbKind::Postgres => quote_ident(kind, user),
    })
}

fn privileges(kind: DbKind, list: &[String]) -> Result<String> {
    if list.is_empty() {
        return Err(Error::Invalid("pick at least one privilege".into()));
    }
    let mut out = Vec::new();
    for p in list {
        let p = p.trim().to_uppercase();
        if !PRIVILEGES.contains(&p.as_str()) {
            return Err(Error::Invalid(format!("unknown privilege {p}")));
        }
        // PostgreSQL has no CREATE VIEW / SHOW VIEW / INDEX / ALTER / DROP / CREATE / EXECUTE on
        // tables.
        if kind == DbKind::Postgres
            && !matches!(
                p.as_str(),
                "ALL PRIVILEGES"
                    | "SELECT"
                    | "INSERT"
                    | "UPDATE"
                    | "DELETE"
                    | "TRUNCATE"
                    | "REFERENCES"
                    | "TRIGGER"
            )
        {
            return Err(Error::Invalid(format!(
                "{p} is not a table privilege on PostgreSQL"
            )));
        }
        if kind == DbKind::Mysql && p == "TRUNCATE" {
            return Err(Error::Invalid(
                "MySQL has no TRUNCATE privilege (it is DROP)".into(),
            ));
        }
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out.join(", "))
}

/// The statements for `change`. `mask` puts `'********'` where the password goes (the preview
/// the page shows); `backslash_escapes` is MySQL's mode (see [`literal`]).
pub fn account_sql(
    kind: DbKind,
    change: &AccountChange,
    backslash_escapes: bool,
    mask: bool,
) -> Result<Vec<String>> {
    let pw = |p: &str| -> Result<String> {
        if p.is_empty() {
            return Err(Error::Invalid("give the account a password".into()));
        }
        if mask {
            Ok("'********'".into())
        } else {
            literal(kind, p, backslash_escapes)
        }
    };
    Ok(match change {
        AccountChange::Create {
            user,
            host,
            password,
        } => {
            let a = account(kind, user, host.as_deref(), backslash_escapes)?;
            match kind {
                DbKind::Mysql => vec![format!("CREATE USER {a} IDENTIFIED BY {};", pw(password)?)],
                DbKind::Postgres => {
                    vec![format!("CREATE ROLE {a} LOGIN PASSWORD {};", pw(password)?)]
                }
            }
        }
        AccountChange::Password {
            user,
            host,
            password,
        } => {
            let a = account(kind, user, host.as_deref(), backslash_escapes)?;
            match kind {
                DbKind::Mysql => vec![format!("ALTER USER {a} IDENTIFIED BY {};", pw(password)?)],
                DbKind::Postgres => vec![format!("ALTER ROLE {a} PASSWORD {};", pw(password)?)],
            }
        }
        AccountChange::Drop { user, host } => {
            let a = account(kind, user, host.as_deref(), backslash_escapes)?;
            match kind {
                DbKind::Mysql => vec![format!("DROP USER {a};")],
                // Its objects in this database pass to the account doing the drop, then its
                // privileges here go: a role that still holds anything cannot be dropped.
                DbKind::Postgres => vec![
                    format!("REASSIGN OWNED BY {a} TO CURRENT_USER;"),
                    format!("DROP OWNED BY {a};"),
                    format!("DROP ROLE {a};"),
                ],
            }
        }
        AccountChange::Grant {
            user,
            host,
            privileges: p,
            namespace,
        }
        | AccountChange::Revoke {
            user,
            host,
            privileges: p,
            namespace,
        } => {
            let grant = matches!(change, AccountChange::Grant { .. });
            let a = account(kind, user, host.as_deref(), backslash_escapes)?;
            let p = privileges(kind, p)?;
            let (verb, dir) = if grant {
                ("GRANT", "TO")
            } else {
                ("REVOKE", "FROM")
            };
            match (kind, namespace.as_deref().filter(|n| !n.trim().is_empty())) {
                (DbKind::Mysql, None) => vec![format!("{verb} {p} ON *.* {dir} {a};")],
                (DbKind::Mysql, Some(ns)) => {
                    vec![format!(
                        "{verb} {p} ON {}.* {dir} {a};",
                        quote_ident(kind, ns)
                    )]
                }
                (DbKind::Postgres, None) => {
                    return Err(Error::Invalid(
                        "pick the schema the privileges are on".into(),
                    ))
                }
                (DbKind::Postgres, Some(ns)) => {
                    let s = quote_ident(kind, ns);
                    let mut v = Vec::new();
                    if grant {
                        v.push(format!("GRANT USAGE ON SCHEMA {s} TO {a};"));
                    }
                    v.push(format!("{verb} {p} ON ALL TABLES IN SCHEMA {s} {dir} {a};"));
                    v
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_masks() {
        let c = AccountChange::Create {
            user: "o'neil".into(),
            host: None,
            password: "pa\\ss'word".into(),
        };
        assert_eq!(
            account_sql(DbKind::Mysql, &c, true, false).unwrap(),
            ["CREATE USER 'o''neil'@'%' IDENTIFIED BY 'pa\\\\ss''word';"]
        );
        // With NO_BACKSLASH_ESCAPES the backslash is a character of its own.
        assert_eq!(
            account_sql(DbKind::Mysql, &c, false, false).unwrap(),
            ["CREATE USER 'o''neil'@'%' IDENTIFIED BY 'pa\\ss''word';"]
        );
        assert_eq!(
            account_sql(DbKind::Postgres, &c, true, false).unwrap(),
            ["CREATE ROLE \"o'neil\" LOGIN PASSWORD E'pa\\\\ss''word';"]
        );
        assert_eq!(
            account_sql(DbKind::Postgres, &c, true, true).unwrap(),
            ["CREATE ROLE \"o'neil\" LOGIN PASSWORD '********';"]
        );
    }

    #[test]
    fn grants_only_known_privileges() {
        let g = |p: &[&str], ns: Option<&str>| AccountChange::Grant {
            user: "app".into(),
            host: Some("10.0.%".into()),
            privileges: p.iter().map(|s| s.to_string()).collect(),
            namespace: ns.map(str::to_string),
        };
        assert_eq!(
            account_sql(
                DbKind::Mysql,
                &g(&["select", "INSERT"], Some("shop")),
                true,
                false
            )
            .unwrap(),
            ["GRANT SELECT, INSERT ON `shop`.* TO 'app'@'10.0.%';"]
        );
        assert_eq!(
            account_sql(
                DbKind::Postgres,
                &g(&["SELECT"], Some("public")),
                true,
                false
            )
            .unwrap(),
            [
                "GRANT USAGE ON SCHEMA \"public\" TO \"app\";",
                "GRANT SELECT ON ALL TABLES IN SCHEMA \"public\" TO \"app\";"
            ]
        );
        assert!(account_sql(
            DbKind::Mysql,
            &g(&["SELECT; DROP DATABASE x"], None),
            true,
            false
        )
        .is_err());
        assert!(account_sql(DbKind::Postgres, &g(&["SELECT"], None), true, false).is_err());
        assert!(account_sql(
            DbKind::Postgres,
            &g(&["CREATE VIEW"], Some("s")),
            true,
            false
        )
        .is_err());
        let r = AccountChange::Revoke {
            user: "app".into(),
            host: None,
            privileges: vec!["ALL PRIVILEGES".into()],
            namespace: None,
        };
        assert_eq!(
            account_sql(DbKind::Mysql, &r, true, false).unwrap(),
            ["REVOKE ALL PRIVILEGES ON *.* FROM 'app'@'%';"]
        );
    }

    #[test]
    fn refuses_bad_input() {
        let bad = AccountChange::Drop {
            user: " ".into(),
            host: None,
        };
        assert!(account_sql(DbKind::Mysql, &bad, true, false).is_err());
        let drop = AccountChange::Drop {
            user: "r".into(),
            host: None,
        };
        assert_eq!(
            account_sql(DbKind::Postgres, &drop, true, false).unwrap(),
            [
                "REASSIGN OWNED BY \"r\" TO CURRENT_USER;",
                "DROP OWNED BY \"r\";",
                "DROP ROLE \"r\";"
            ]
        );
        let empty = AccountChange::Password {
            user: "a".into(),
            host: None,
            password: String::new(),
        };
        assert!(account_sql(DbKind::Postgres, &empty, true, false).is_err());
    }

    #[test]
    fn names_are_kept_exactly_and_follow_the_escape_mode() {
        // An existing role named with a space is not another role.
        let drop = AccountChange::Drop {
            user: " app".into(),
            host: None,
        };
        assert_eq!(
            account_sql(DbKind::Postgres, &drop, true, false).unwrap()[2],
            "DROP ROLE \" app\";"
        );
        let d = AccountChange::Drop {
            user: "a\\b".into(),
            host: Some("h\\%".into()),
        };
        assert_eq!(
            account_sql(DbKind::Mysql, &d, true, false).unwrap(),
            ["DROP USER 'a\\\\b'@'h\\\\%';"]
        );
        assert_eq!(
            account_sql(DbKind::Mysql, &d, false, false).unwrap(),
            ["DROP USER 'a\\b'@'h\\%';"]
        );
    }
}
