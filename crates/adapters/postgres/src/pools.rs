//! The primary pool and the read pool, and their readiness checks.

use std::num::NonZeroU32;
use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::unavailable::ACQUIRE_TIMEOUT;
use crate::{DatabaseUnavailable, DatabaseUrl};

/// How long Postgres lets a statement on a serving pool run before it cancels it: the request
/// timeout, so that no statement outlives its request for long (spec §6.1, §6.4).
const STATEMENT_TIMEOUT: &str = "5s";

/// When an idle connection closes, and when every connection does. sqlx's defaults, written out
/// so that an upgrade cannot change them unseen.
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_LIFETIME: Duration = Duration::from_secs(30 * 60);

/// The primary pool, for writes and for the reads that follow a write, and the read pool, for
/// the read models: a replica when there is one (ADR-0007). When the two URLs are equal, one
/// pool serves both roles.
///
/// The pools are built without connecting: a database that is down at start is a readiness
/// failure, not a start-up one, and the service recovers on its own when it comes back. A pool
/// keeps no connection open while idle, and getting one, opening it included, gives up after 2
/// seconds; until then sqlx retries a connection the database refuses.
///
/// Cloning shares the pools. Like the connect options they hold, they have no `Debug`.
#[derive(Clone)]
pub struct Pools {
    primary: PgPool,
    /// `None` when the read URL is the primary's.
    read: Option<PgPool>,
}

impl Pools {
    /// Both pools, each of at most `max_connections`, whose connections name themselves
    /// `application_name` to Postgres, so that `pg_stat_activity` shows who holds them. It must
    /// be called inside a tokio runtime: each pool starts a task there that closes the
    /// connections idle or old enough.
    pub fn new(
        application_name: &str,
        primary: &DatabaseUrl,
        read: &DatabaseUrl,
        max_connections: NonZeroU32,
    ) -> Self {
        let pool = |url: &DatabaseUrl| {
            PgPoolOptions::new()
                .max_connections(max_connections.get())
                .min_connections(0)
                .acquire_timeout(ACQUIRE_TIMEOUT)
                .idle_timeout(IDLE_TIMEOUT)
                .max_lifetime(MAX_LIFETIME)
                // Sent when the connection opens, so they hold from its first statement.
                .connect_lazy_with(
                    url.connect_options()
                        .application_name(application_name)
                        .options([("statement_timeout", STATEMENT_TIMEOUT)]),
                )
        };
        Self {
            primary: pool(primary),
            read: (!read.is_same_as(primary)).then(|| pool(read)),
        }
    }

    /// The pool for writes, and for the reads that follow a write.
    pub fn primary(&self) -> &PgPool {
        &self.primary
    }

    /// The pool for the read models; the primary pool when the two URLs are equal.
    pub fn read(&self) -> &PgPool {
        self.read.as_ref().unwrap_or(&self.primary)
    }

    /// Whether the primary pool serves the reads too, so that one readiness check covers both.
    pub fn read_is_primary(&self) -> bool {
        self.read.is_none()
    }

    /// Whether the primary database answers: one query through its pool, which gives up after
    /// 2 seconds without a connection.
    pub async fn check_primary(&self) -> Result<(), DatabaseUnavailable> {
        check(&self.primary).await
    }

    /// Whether the read database answers, as [`check_primary`](Self::check_primary) does.
    pub async fn check_read(&self) -> Result<(), DatabaseUnavailable> {
        check(self.read()).await
    }

    /// Closes both pools: waits for every connection in use to come back, then closes each one,
    /// so that Postgres sees it end. Getting a connection fails from then on.
    pub async fn close(&self) {
        self.primary.close().await;
        if let Some(read) = &self.read {
            read.close().await;
        }
    }
}

/// One compile-time checked query, the first in `.sqlx/`.
async fn check(pool: &PgPool) -> Result<(), DatabaseUnavailable> {
    sqlx::query_scalar!("SELECT 1")
        .fetch_one(pool)
        .await
        .map(drop)
        .map_err(|error| DatabaseUnavailable::from(&error))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A database that nobody connects to: the pools are built without connecting.
    const PRIMARY: &str = "postgres://barras:hunter2-secret@127.0.0.1:5999/barras";
    const REPLICA: &str = "postgres://barras:hunter2-secret@replica.example:5432/barras";

    fn url(text: &str) -> DatabaseUrl {
        DatabaseUrl::parse(text).unwrap()
    }

    fn size(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap()
    }

    #[tokio::test]
    async fn each_pool_has_the_limits_of_the_spec() {
        let pools = Pools::new("barras-api", &url(PRIMARY), &url(REPLICA), size(7));

        for pool in [pools.primary(), pools.read()] {
            let options = pool.options();
            assert_eq!(options.get_max_connections(), 7);
            assert_eq!(options.get_min_connections(), 0);
            assert_eq!(options.get_acquire_timeout(), Duration::from_secs(2));
            assert_eq!(options.get_idle_timeout(), Some(Duration::from_secs(600)));
            assert_eq!(options.get_max_lifetime(), Some(Duration::from_secs(1800)));
        }
    }

    #[tokio::test]
    async fn each_connection_names_itself_and_asks_for_the_statement_timeout() {
        let pools = Pools::new("barras-api", &url(PRIMARY), &url(REPLICA), size(7));

        for pool in [pools.primary(), pools.read()] {
            let options = pool.connect_options();
            assert_eq!(options.get_application_name(), Some("barras-api"));
            assert_eq!(options.get_options(), Some("-c statement_timeout=5s"));
        }
    }

    #[tokio::test]
    async fn each_pool_connects_where_its_url_says() {
        let pools = Pools::new("barras-api", &url(PRIMARY), &url(REPLICA), size(7));

        assert_eq!(pools.primary().connect_options().get_host(), "127.0.0.1");
        assert_eq!(pools.read().connect_options().get_host(), "replica.example");
    }

    #[tokio::test]
    async fn equal_urls_make_one_pool_and_different_ones_two() {
        let one = Pools::new("barras-api", &url(PRIMARY), &url(PRIMARY), size(7));
        let two = Pools::new("barras-api", &url(PRIMARY), &url(REPLICA), size(7));

        assert!(one.read_is_primary());
        assert!(!two.read_is_primary());
    }

    #[tokio::test]
    async fn no_connection_is_opened_until_one_is_needed() {
        let pools = Pools::new("barras-api", &url(PRIMARY), &url(REPLICA), size(7));

        assert_eq!(pools.primary().size(), 0);
        assert_eq!(pools.read().size(), 0);
    }
}
