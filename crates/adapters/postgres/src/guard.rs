//! The local-database guard: seeds and resets change only a database on this machine.

use std::fmt;
use std::io;
use std::net::{IpAddr, ToSocketAddrs};

use sqlx::postgres::PgConnectOptions;

use crate::DatabaseUrl;

/// Allows the database `url` names only when it is on this machine (spec §9.3): a Unix socket,
/// an address of `127.0.0.0/8` or `::1`, or `localhost` resolving to loopback addresses alone.
///
/// Every other host is refused, a name included even when it resolves to this machine: only
/// `localhost` names this machine by convention, and what another name resolves to is up to
/// whoever answers for it.
///
/// It judges the target sqlx connects to, read as sqlx reads the URL: the socket before the
/// host, since sqlx connects to the socket whatever the host holds (plan, appendix A.5). It
/// opens no connection. `APP_ENV` is the composition root's to check.
pub fn ensure_local(url: &DatabaseUrl) -> Result<(), NotLocal> {
    judge(&url.connect_options(), resolve)
}

/// The addresses `host` resolves to, through the system's resolver, as the connection resolves
/// them. It blocks while the resolver answers, which for `localhost`, the one name the guard
/// resolves, comes from the hosts file.
fn resolve(host: &str, port: u16) -> io::Result<Vec<IpAddr>> {
    Ok((host, port)
        .to_socket_addrs()?
        .map(|address| address.ip())
        .collect())
}

/// The guard's decision on where `options` connect, with `resolve` answering for `localhost`.
fn judge(
    options: &PgConnectOptions,
    resolve: impl FnOnce(&str, u16) -> io::Result<Vec<IpAddr>>,
) -> Result<(), NotLocal> {
    // sqlx takes a host that starts with `/` for the folder of a socket, as it takes the socket.
    if options.get_socket().is_some() || options.get_host().starts_with('/') {
        return Ok(());
    }
    let host = options.get_host();
    // sqlx takes `a,b` for one name and tries no host of the list (plan, appendix A.5).
    if host.contains(',') {
        return Err(NotLocal::SeveralHosts);
    }
    if let Ok(address) = host.parse::<IpAddr>() {
        return if address.is_loopback() {
            Ok(())
        } else {
            Err(NotLocal::OtherAddress)
        };
    }
    if !host.eq_ignore_ascii_case("localhost") {
        return Err(NotLocal::OtherName);
    }
    match resolve(host, options.get_port()) {
        Ok(addresses) if addresses.is_empty() => Err(NotLocal::LocalhostUnresolved),
        Ok(addresses) if addresses.iter().all(IpAddr::is_loopback) => Ok(()),
        Ok(_) => Err(NotLocal::LocalhostElsewhere),
        // The resolver's text says nothing the reason does not.
        Err(_) => Err(NotLocal::LocalhostUnresolved),
    }
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
}

impl fmt::Display for NotLocal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let why = match self {
            Self::OtherName => "its host is a name other than localhost",
            Self::OtherAddress => "its host is an address outside 127.0.0.0/8 and ::1",
            Self::SeveralHosts => "its URL names several hosts",
            Self::LocalhostElsewhere => {
                "localhost resolves to an address outside 127.0.0.0/8 and ::1"
            }
            Self::LocalhostUnresolved => "localhost does not resolve",
        };
        write!(f, "the database may be on another machine: {why}")
    }
}

impl std::error::Error for NotLocal {}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the tests keep secret; no refusal may show it.
    const PASSWORD: &str = "hunter2-secret";

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

    fn verdict(
        url: &str,
        resolve: impl FnOnce(&str, u16) -> io::Result<Vec<IpAddr>>,
    ) -> Result<(), NotLocal> {
        let url = DatabaseUrl::parse(url).unwrap();
        judge(&url.connect_options(), resolve)
    }

    /// Options as sqlx builds them, for a target no URL of the rule can name.
    fn options_with_host(host: &str) -> PgConnectOptions {
        PgConnectOptions::new_without_pgpass().host(host)
    }

    #[test]
    fn a_loopback_address_or_a_socket_is_allowed_without_resolving_anything() {
        for url in [
            format!("postgres://barras:{PASSWORD}@127.0.0.1:5433/barras"),
            format!("postgres://barras:{PASSWORD}@127.255.0.9:5432/barras"),
            // A socket's folder, percent-encoded as sqlx reads it.
            format!("postgres://barras:{PASSWORD}@%2Fvar%2Frun%2Fpostgresql:5432/barras"),
        ] {
            assert_eq!(verdict(&url, unused), Ok(()), "{url}");
        }
    }

    #[test]
    fn localhost_resolving_to_loopback_addresses_is_allowed_whatever_its_case() {
        for url in [
            format!("postgres://barras:{PASSWORD}@localhost:5433/barras"),
            format!("postgres://barras:{PASSWORD}@LocalHost:5433/barras"),
        ] {
            assert_eq!(verdict(&url, loopback), Ok(()), "{url}");
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

        assert_eq!(judge(&options, unused), Ok(()));
    }

    #[test]
    fn a_host_that_names_a_folder_is_a_socket() {
        assert_eq!(
            judge(&options_with_host("/var/run/postgresql"), unused),
            Ok(())
        );
    }

    #[test]
    fn of_the_ipv6_addresses_only_the_loopback_one_is_allowed() {
        // No URL of the rule names an IPv6 address, but a host can hold one.
        assert_eq!(judge(&options_with_host("::1"), unused), Ok(()));
        for host in ["fe80::1", "::", "::ffff:127.0.0.1"] {
            assert_eq!(
                judge(&options_with_host(host), unused),
                Err(NotLocal::OtherAddress),
                "{host}"
            );
        }
    }

    #[test]
    fn a_host_that_holds_several_hosts_is_refused() {
        for host in ["127.0.0.1,10.0.0.1", "localhost,db.example.com"] {
            assert_eq!(
                judge(&options_with_host(host), unused),
                Err(NotLocal::SeveralHosts),
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
    fn a_name_that_does_not_resolve_is_refused_without_waiting_on_the_resolver() {
        // `.invalid` never resolves (RFC 6761); the guard refuses it as it refuses any name but
        // localhost, before asking.
        let url = DatabaseUrl::parse(&format!(
            "postgres://barras:{PASSWORD}@db.invalid:5432/barras"
        ))
        .unwrap();

        assert_eq!(ensure_local(&url), Err(NotLocal::OtherName));
    }

    #[test]
    fn localhost_is_this_machine_through_the_system_resolver() {
        let url = DatabaseUrl::parse(&format!(
            "postgres://barras:{PASSWORD}@localhost:5433/barras"
        ))
        .unwrap();

        assert_eq!(ensure_local(&url), Ok(()));
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
}
