//! The local-database guard: seeds and resets change only a database on this machine, and only
//! once the server that answers has shown that it is the local one.

use std::fmt;
use std::io;
use std::net::{IpAddr, ToSocketAddrs};

use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, PgConnection};

use crate::{DatabaseUnavailable, DatabaseUrl};

/// The `cluster_name` of the Postgres that `seed` and `reset` may change (plan, refinement 15):
/// the Postgres of `compose.yaml`, the test containers and the justfile's throwaway database
/// start with `-c cluster_name=barras-local`. A server takes the setting only when it starts, so
/// no session can give it to one, and a server elsewhere, such as staging, does not carry it. A
/// test fails when `compose.yaml` or the justfile names another.
pub(crate) const LOCAL_CLUSTER_NAME: &str = "barras-local";

/// Allows the database `url` names only when it is on this machine (spec §9.3): a Unix socket,
/// an address of `127.0.0.0/8` or `::1`, or `localhost` resolving to loopback addresses alone.
/// Returns it aimed at what was judged, which is where a run connects: `localhost` is resolved
/// here, once, so that no later answer of the resolver can send the run elsewhere.
///
/// Every other host is refused, a name included even when it resolves to this machine: only
/// `localhost` names this machine by convention, and what another name resolves to is up to
/// whoever answers for it.
///
/// It judges the target sqlx connects to, read as sqlx reads the URL: the socket before the
/// host, since sqlx connects to the socket whatever the host holds (plan, appendix A.5). It
/// opens no connection. A port of this machine can still lead to another one, so each
/// connection of a run also asks the server, before it is used, whether it is the local one
/// ([`LocalDatabase::connect`]). `APP_ENV` is the composition root's to check.
pub(crate) fn ensure_local(url: &DatabaseUrl) -> Result<LocalDatabase, NotLocal> {
    judge(url.connect_options(), resolve)
}

/// The addresses `host` resolves to, through the system's resolver, in the order a connection to
/// the name would try them. It blocks while the resolver answers, which for `localhost`, the one
/// name the guard resolves, comes from the hosts file.
fn resolve(host: &str, port: u16) -> io::Result<Vec<IpAddr>> {
    Ok((host, port)
        .to_socket_addrs()?
        .map(|address| address.ip())
        .collect())
}

/// The guard's decision on where `options` connect, with `resolve` answering for `localhost`.
fn judge(
    options: PgConnectOptions,
    resolve: impl FnOnce(&str, u16) -> io::Result<Vec<IpAddr>>,
) -> Result<LocalDatabase, NotLocal> {
    // sqlx takes a host that starts with `/` for the folder of a socket, as it takes the socket.
    if options.get_socket().is_some() || options.get_host().starts_with('/') {
        return Ok(LocalDatabase::at(options));
    }
    let host = options.get_host();
    // sqlx takes `a,b` for one name and tries no host of the list (plan, appendix A.5).
    if host.contains(',') {
        return Err(NotLocal::SeveralHosts);
    }
    if let Ok(address) = host.parse::<IpAddr>() {
        return if address.is_loopback() {
            Ok(LocalDatabase::at(options))
        } else {
            Err(NotLocal::OtherAddress)
        };
    }
    if !host.eq_ignore_ascii_case("localhost") {
        return Err(NotLocal::OtherName);
    }
    let addresses = match resolve(host, options.get_port()) {
        Ok(addresses) => addresses,
        // The resolver's text says nothing the reason does not.
        Err(_) => return Err(NotLocal::LocalhostUnresolved),
    };
    if !addresses.iter().all(IpAddr::is_loopback) {
        return Err(NotLocal::LocalhostElsewhere);
    }
    let mut judged: Vec<IpAddr> = Vec::new();
    for address in addresses {
        if !judged.contains(&address) {
            judged.push(address);
        }
    }
    let mut aimed = judged
        .iter()
        .map(|address| options.clone().host(&address.to_string()));
    match aimed.next() {
        Some(first) => Ok(LocalDatabase {
            first,
            others: aimed.collect(),
        }),
        None => Err(NotLocal::LocalhostUnresolved),
    }
}

/// A database the guard judged to be on this machine, aimed at what it judged: a run connects
/// there, and never asks the resolver again (plan, refinement 15). It holds the password, so it
/// has no `Debug`.
pub(crate) struct LocalDatabase {
    /// The URL's options as they are, for a socket or a loopback address; for `localhost`, aimed
    /// at the first address it resolved to.
    first: PgConnectOptions,
    /// Aimed at each other address `localhost` resolved to, in the resolver's order.
    others: Vec<PgConnectOptions>,
}

impl LocalDatabase {
    fn at(options: PgConnectOptions) -> Self {
        Self {
            first: options,
            others: Vec::new(),
        }
    }

    /// The database the URL names. A URL of the rule names one; without it, Postgres would take
    /// the user's name.
    pub(crate) fn database(&self) -> &str {
        self.first
            .get_database()
            .unwrap_or(self.first.get_username())
    }

    /// The database `database` on the same server, at the same addresses.
    pub(crate) fn with_database(&self, database: &str) -> Self {
        Self {
            first: self.first.clone().database(database),
            others: self
                .others
                .iter()
                .map(|options| options.clone().database(database))
                .collect(),
        }
    }

    /// A connection of its own to the database, once the server that answered has shown that it
    /// started as the local one ([`ensure_local_server`]): the only way to one, so a run uses no
    /// connection that was not asked. Each is asked, never only a run's first: a port of this
    /// machine can lead to another server on each connection, as a proxy that takes turns
    /// between servers does, so what one server answered says nothing of the next.
    ///
    /// A connection whose server was not let through is closed before this returns.
    pub(crate) async fn connect(&self) -> Result<PgConnection, ServerCheckFailed> {
        let mut connection = self
            .reach()
            .await
            .map_err(|error| ServerCheckFailed::Unavailable(DatabaseUnavailable::from(&error)))?;
        match ensure_local_server(&mut connection).await {
            Ok(()) => Ok(connection),
            Err(failure) => {
                if let Err(error) = connection.close().await {
                    // A connection that cannot close cleanly is dropped instead, which closes its
                    // socket: the session ends either way, so the refusal stands.
                    tracing::warn!(
                        error = %DatabaseUnavailable::from(&error),
                        "the connection that asked the server who it is did not close cleanly"
                    );
                }
                Err(failure)
            }
        }
    }

    /// A connection to the first of the judged addresses that can be reached: a connection to
    /// `localhost` tries the addresses the name resolves to in turn, as this does. Nothing has
    /// asked the server who it is yet.
    async fn reach(&self) -> Result<PgConnection, sqlx::Error> {
        let mut connected = PgConnection::connect_with(&self.first).await;
        for options in &self.others {
            if !matches!(connected, Err(sqlx::Error::Io(_))) {
                break;
            }
            connected = PgConnection::connect_with(options).await;
        }
        connected
    }
}

/// Refuses the server `connection` reached unless it started as the local one, with the
/// `cluster_name` of [`LOCAL_CLUSTER_NAME`] (plan, refinement 15). It is the first question on
/// every connection of a run, before anything changes: the guard allows a port of this machine,
/// and such a port can be the first hop to another machine, as `ssh -L`, `kubectl port-forward`
/// or a cloud SQL proxy makes it.
async fn ensure_local_server(connection: &mut PgConnection) -> Result<(), ServerCheckFailed> {
    let name = sqlx::query_scalar!(r#"SELECT current_setting('cluster_name') AS "name!""#)
        .fetch_one(connection)
        .await
        .map_err(|error| ServerCheckFailed::Unavailable(DatabaseUnavailable::from(&error)))?;
    if name == LOCAL_CLUSTER_NAME {
        Ok(())
    } else {
        Err(ServerCheckFailed::NotLocal(NotLocal::OtherServer))
    }
}

/// Why a run got no connection to the local server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerCheckFailed {
    /// The server that answered did not start as the local one.
    NotLocal(NotLocal),
    /// No server could be reached, or the one that answered could not be asked.
    Unavailable(DatabaseUnavailable),
}

/// Why the guard refused a database: one sentence of a fixed set, which never quotes the URL,
/// since it holds the password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotLocal {
    /// Its host is a name other than `localhost`, which the guard never resolves.
    OtherName,
    /// Its host is an address outside `127.0.0.0/8` and `::1`.
    OtherAddress,
    /// Its host holds several hosts, which sqlx takes for one name that no resolver answers.
    SeveralHosts,
    /// `localhost` resolves to an address outside `127.0.0.0/8` and `::1`.
    LocalhostElsewhere,
    /// `localhost` does not resolve.
    LocalhostUnresolved,
    /// The server that answered at the address did not start as the local one: its
    /// `cluster_name` is not that of the local database. The address can be a relay to another
    /// machine.
    OtherServer,
}

impl fmt::Display for NotLocal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the database may be on another machine: ")?;
        match self {
            Self::OtherName => f.write_str("its host is a name other than localhost"),
            Self::OtherAddress => f.write_str("its host is an address outside 127.0.0.0/8 and ::1"),
            Self::SeveralHosts => f.write_str("its URL names several hosts"),
            Self::LocalhostElsewhere => {
                f.write_str("localhost resolves to an address outside 127.0.0.0/8 and ::1")
            }
            Self::LocalhostUnresolved => f.write_str("localhost does not resolve"),
            Self::OtherServer => write!(
                f,
                "the server that answered does not have cluster_name set to \
                 {LOCAL_CLUSTER_NAME}, as the Postgres of compose.yaml has; a Postgres of your own \
                 takes it from the line cluster_name = '{LOCAL_CLUSTER_NAME}' in its \
                 postgresql.conf, and a restart"
            ),
        }
    }
}

impl std::error::Error for NotLocal {}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::iter;
    use std::path::Path;

    use super::*;

    /// What the tests keep secret; no refusal may show it.
    const PASSWORD: &str = "hunter2-secret";

    impl LocalDatabase {
        /// Where a run connects, in the order it tries: a socket's path, or a host.
        fn targets(&self) -> Vec<String> {
            iter::once(&self.first)
                .chain(&self.others)
                .map(|options| match options.get_socket() {
                    Some(socket) => socket.display().to_string(),
                    None => options.get_host().to_owned(),
                })
                .collect()
        }
    }

    /// What `localhost` resolves to on a machine set up as usual.
    fn loopback(_host: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
        Ok(vec![
            IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1]),
            IpAddr::from([127, 0, 0, 1]),
        ])
    }

    /// A resolver the guard must not call: it judges every host but `localhost` as written.
    fn unused(host: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
        panic!("the guard resolved {host}");
    }

    /// Where the guard lets a run connect to the database `url` names, or why it does not.
    fn verdict(
        url: &str,
        resolve: impl FnOnce(&str, u16) -> io::Result<Vec<IpAddr>>,
    ) -> Result<Vec<String>, NotLocal> {
        let url = DatabaseUrl::parse(url).unwrap();
        judge(url.connect_options(), resolve).map(|local| local.targets())
    }

    /// Options as sqlx builds them, for a target no URL of the rule can name.
    fn options_with_host(host: &str) -> PgConnectOptions {
        PgConnectOptions::new_without_pgpass().host(host)
    }

    fn targets(list: &[&str]) -> Result<Vec<String>, NotLocal> {
        Ok(list.iter().map(|target| (*target).to_owned()).collect())
    }

    #[test]
    fn a_loopback_address_or_a_socket_is_allowed_as_it_is_without_resolving_anything() {
        for (url, target) in [
            (
                format!("postgres://barras:{PASSWORD}@127.0.0.1:5433/barras"),
                "127.0.0.1",
            ),
            (
                format!("postgres://barras:{PASSWORD}@127.255.0.9:5432/barras"),
                "127.255.0.9",
            ),
            // A socket's folder, percent-encoded as sqlx reads it.
            (
                format!("postgres://barras:{PASSWORD}@%2Fvar%2Frun%2Fpostgresql:5432/barras"),
                "/var/run/postgresql",
            ),
        ] {
            assert_eq!(verdict(&url, unused), targets(&[target]), "{url}");
        }
    }

    #[test]
    fn localhost_resolving_to_loopback_addresses_is_allowed_whatever_its_case() {
        for url in [
            format!("postgres://barras:{PASSWORD}@localhost:5433/barras"),
            format!("postgres://barras:{PASSWORD}@LocalHost:5433/barras"),
        ] {
            assert!(verdict(&url, loopback).is_ok(), "{url}");
        }
    }

    #[test]
    fn localhost_is_resolved_once_and_a_run_connects_to_the_addresses_judged() {
        // In the resolver's order, as a connection to the name would try them, each once.
        let url = format!("postgres://barras:{PASSWORD}@localhost:5433/barras");
        let twice = |_: &str, _: u16| {
            Ok(vec![
                IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1]),
                IpAddr::from([127, 0, 0, 1]),
                IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1]),
            ])
        };

        assert_eq!(verdict(&url, twice), targets(&["::1", "127.0.0.1"]));
    }

    #[test]
    fn another_database_of_the_server_is_at_the_same_addresses() {
        let url = DatabaseUrl::parse(&format!(
            "postgres://barras:{PASSWORD}@localhost:5433/barras"
        ))
        .unwrap();
        let local = judge(url.connect_options(), loopback).unwrap();

        let maintenance = local.with_database("postgres");

        assert_eq!(local.database(), "barras");
        assert_eq!(maintenance.database(), "postgres");
        assert_eq!(maintenance.targets(), ["::1", "127.0.0.1"]);
        for options in iter::once(&maintenance.first).chain(&maintenance.others) {
            assert_eq!(options.get_database(), Some("postgres"));
            assert_eq!(options.get_port(), 5433);
        }
    }

    #[test]
    fn any_other_host_is_refused_without_being_resolved() {
        for (url, reason) in [
            ("postgres://u:p@db.example.com:5432/db", NotLocal::OtherName),
            // A name of this machine's own may resolve to it, and is refused all the same.
            ("postgres://u:p@my-laptop:5432/db", NotLocal::OtherName),
            ("postgres://u:p@localhost.:5432/db", NotLocal::OtherName),
            (
                "postgres://u:p@localhost.example.com:5432/db",
                NotLocal::OtherName,
            ),
            // sqlx connects to the host as the URL writes it, percent sign included.
            ("postgres://u:p@local%68ost:5432/db", NotLocal::OtherName),
            // A shortened address is a name to the parser, and is refused rather than read.
            ("postgres://u:p@127.1:5432/db", NotLocal::OtherName),
            ("postgres://u:p@10.0.0.5:5432/db", NotLocal::OtherAddress),
            (
                "postgres://u:p@192.168.1.20:5432/db",
                NotLocal::OtherAddress,
            ),
            ("postgres://u:p@128.0.0.1:5432/db", NotLocal::OtherAddress),
            // It reaches this machine on most systems, but is no loopback address.
            ("postgres://u:p@0.0.0.0:5432/db", NotLocal::OtherAddress),
        ] {
            assert_eq!(verdict(url, unused), Err(reason), "{url}");
        }
    }

    #[test]
    fn the_socket_is_read_before_the_host() {
        // With a socket in the URL, the host holds a default or PGHOST's value, which sqlx
        // never connects to.
        let options = options_with_host("db.example.com").socket("/tmp/sockets");

        assert_eq!(
            judge(options, unused).map(|local| local.targets()),
            targets(&["/tmp/sockets"])
        );
    }

    #[test]
    fn a_host_that_names_a_folder_is_a_socket() {
        assert_eq!(
            judge(options_with_host("/var/run/postgresql"), unused).map(|local| local.targets()),
            targets(&["/var/run/postgresql"])
        );
    }

    #[test]
    fn of_the_ipv6_addresses_only_the_loopback_one_is_allowed() {
        // No URL of the rule names an IPv6 address, but a host can hold one.
        assert_eq!(
            judge(options_with_host("::1"), unused).map(|local| local.targets()),
            targets(&["::1"])
        );
        for host in ["fe80::1", "::", "::ffff:127.0.0.1"] {
            assert_eq!(
                judge(options_with_host(host), unused).err(),
                Some(NotLocal::OtherAddress),
                "{host}"
            );
        }
    }

    #[test]
    fn a_host_that_holds_several_hosts_is_refused() {
        for host in ["127.0.0.1,10.0.0.1", "localhost,db.example.com"] {
            assert_eq!(
                judge(options_with_host(host), unused).err(),
                Some(NotLocal::SeveralHosts),
                "{host}"
            );
        }
    }

    #[test]
    fn localhost_resolving_to_any_other_address_is_refused() {
        let url = "postgres://u:p@localhost:5432/db";
        let mixed = |_: &str, _: u16| {
            Ok(vec![
                IpAddr::from([127, 0, 0, 1]),
                IpAddr::from([192, 168, 1, 20]),
            ])
        };

        assert_eq!(verdict(url, mixed), Err(NotLocal::LocalhostElsewhere));
    }

    #[test]
    fn localhost_that_does_not_resolve_is_refused() {
        let url = "postgres://u:p@localhost:5432/db";
        let failing = |_: &str, _: u16| Err(io::Error::from(io::ErrorKind::NotFound));
        let empty = |_: &str, _: u16| Ok(Vec::new());

        assert_eq!(verdict(url, failing), Err(NotLocal::LocalhostUnresolved));
        assert_eq!(verdict(url, empty), Err(NotLocal::LocalhostUnresolved));
    }

    #[test]
    fn a_name_that_does_not_resolve_is_refused_for_not_being_localhost() {
        // `.invalid` never resolves (RFC 6761): through the system's resolver too, the guard
        // refuses it as any name but localhost.
        let url = DatabaseUrl::parse(&format!(
            "postgres://barras:{PASSWORD}@db.invalid:5432/barras"
        ))
        .unwrap();

        assert_eq!(ensure_local(&url).err(), Some(NotLocal::OtherName));
    }

    #[test]
    fn localhost_is_this_machine_through_the_system_resolver() {
        let url = DatabaseUrl::parse(&format!(
            "postgres://barras:{PASSWORD}@localhost:5433/barras"
        ))
        .unwrap();

        let judged = ensure_local(&url).map(|local| local.targets());

        let judged = judged.unwrap();
        assert!(!judged.is_empty());
        for address in judged {
            assert!(
                address.parse::<IpAddr>().unwrap().is_loopback(),
                "{address}"
            );
        }
    }

    #[test]
    fn each_refusal_is_a_fixed_sentence() {
        for (reason, sentence) in [
            (
                NotLocal::OtherName,
                "the database may be on another machine: its host is a name other than localhost",
            ),
            (
                NotLocal::OtherAddress,
                "the database may be on another machine: its host is an address outside \
                 127.0.0.0/8 and ::1",
            ),
            (
                NotLocal::SeveralHosts,
                "the database may be on another machine: its URL names several hosts",
            ),
            (
                NotLocal::LocalhostElsewhere,
                "the database may be on another machine: localhost resolves to an address \
                 outside 127.0.0.0/8 and ::1",
            ),
            (
                NotLocal::LocalhostUnresolved,
                "the database may be on another machine: localhost does not resolve",
            ),
            (
                NotLocal::OtherServer,
                "the database may be on another machine: the server that answered does not have \
                 cluster_name set to barras-local, as the Postgres of compose.yaml has; a \
                 Postgres of your own takes it from the line cluster_name = 'barras-local' in its \
                 postgresql.conf, and a restart",
            ),
        ] {
            assert_eq!(reason.to_string(), sentence);
        }
    }

    #[test]
    fn no_refusal_quotes_the_url() {
        for (url, host) in [
            (
                format!("postgres://barras:{PASSWORD}@db.example.com:5432/barras"),
                "db.example.com",
            ),
            (
                format!("postgres://barras:{PASSWORD}@10.0.0.5:5432/barras"),
                "10.0.0.5",
            ),
        ] {
            let reason = verdict(&url, unused).unwrap_err();

            for shown in [reason.to_string(), format!("{reason:?}")] {
                assert!(!shown.contains(PASSWORD), "{shown}");
                assert!(!shown.contains(host), "{shown}");
            }
        }
    }

    /// The value of each `cluster_name=` that `text` sets outside its comment lines.
    fn cluster_names_set_in(text: &str) -> Vec<&str> {
        const KEY: &str = "cluster_name=";
        text.lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .flat_map(|line| {
                line.match_indices(KEY)
                    .map(move |(at, _)| &line[at + KEY.len()..])
            })
            .map(|value| {
                let end = value
                    .find(|character: char| {
                        !(character.is_ascii_alphanumeric() || "-_".contains(character))
                    })
                    .unwrap_or(value.len());
                &value[..end]
            })
            .collect()
    }

    #[test]
    fn compose_yaml_and_the_justfile_start_postgres_with_the_local_cluster_name() {
        // Were they to differ, `seed` and `reset` would refuse the local database itself.
        for file in ["compose.yaml", "justfile"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .join(file);
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

            let names = cluster_names_set_in(&text);

            assert!(
                !names.is_empty(),
                "{file} starts no Postgres with a cluster_name"
            );
            assert!(
                names.iter().all(|name| *name == LOCAL_CLUSTER_NAME),
                "{file}: {names:?}"
            );
        }
    }

    #[test]
    fn a_cluster_name_is_read_up_to_where_its_value_ends() {
        let text = "# cluster_name=commented\n    command: [\"postgres\", \"-c\", \
                    \"cluster_name=barras-local\"]\n  postgres -c fsync=off -c cluster_name=a_b)\"";

        assert_eq!(cluster_names_set_in(text), ["barras-local", "a_b"]);
    }
}
