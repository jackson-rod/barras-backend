//! A Postgres container per test, from the image of the `postgres` service of `compose.yaml`, and
//! a relay that reaches one through a port of this machine.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::process::Command;

use testcontainers::bollard::models::{HostConfig, PortBinding};
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, Image, ImageExt, TestcontainersError};

use crate::repository_file;

/// The user, the password and the database of every container: a container is thrown away
/// with its test, so these protect nothing.
const USER: &str = "barras";
const PASSWORD: &str = "barras";
const DATABASE: &str = "barras";

/// Where Postgres listens inside the container, and the relay inside its own.
const PORT: ContainerPort = ContainerPort::Tcp(5432);

/// The label every test container carries, which finds one that a test killed with SIGKILL
/// left behind.
const LABEL: (&str, &str) = ("barras-test", "1");

/// Docker's default network, named outright: a relay finds its target there by the name of the
/// target's network mode, which an engine may otherwise report as `default`, a name no network
/// answers to.
const NETWORK: &str = "bridge";

/// What Postgres prints once it accepts connections.
const READY: &str = "database system is ready to accept connections";

/// The setting the local database starts with, which `seed` and `reset` of the postgres adapter
/// require of a server before they change anything on it (plan, refinement 15). The adapter
/// holds the name, as `LOCAL_CLUSTER_NAME`, and fails a test when `compose.yaml` names another;
/// the testkit cannot take it from there, since the adapter takes the testkit as a
/// dev-dependency, so the adapter's `int_` tests are what fail when this one differs.
const LOCAL_CLUSTER_SETTING: &str = "cluster_name=barras-local";

/// The relay, in Perl, which the Postgres image carries: it listens on port 5432 and copies the
/// bytes of each connection both ways, in a process per connection, to the address of one of its
/// arguments, each in turn: with one, as `ssh -L` does; with several, as a proxy that takes
/// turns between servers does.
const RELAY: &str = r#"
use IO::Select;
use IO::Socket::INET;
$SIG{CHLD} = "IGNORE";
my $listener = IO::Socket::INET->new(LocalPort => 5432, Listen => 16, ReuseAddr => 1)
    or die "cannot listen: $!";
print STDERR "relay ready\n";
my $turn = 0;
while (1) {
    my $client = $listener->accept or next;
    my $target = $ARGV[$turn++ % @ARGV];
    if (fork) { close $client; next; }
    my $server = IO::Socket::INET->new(PeerAddr => $target) or exit 1;
    my $both = IO::Select->new($client, $server);
    while (1) {
        for my $from ($both->can_read) {
            my $to = $from == $client ? $server : $client;
            my $read = sysread($from, my $bytes, 65536) or exit 0;
            for (my $sent = 0; $sent < $read;) {
                $sent += syswrite($to, $bytes, $read - $sent, $sent) // exit 0;
            }
        }
    }
}
"#;

/// What the relay prints once it listens.
const RELAY_READY: &str = "relay ready";

/// A Postgres 18 container that one test started, with an empty database `barras`. Each test
/// starts its own, so no test sees another's data, and a container held in a `static` would
/// never be removed: nextest runs every test in a process of its own.
///
/// Dropping it removes the container, also while a failed test unwinds; it must be dropped
/// inside the tokio runtime that started it. A test stopped with SIGTERM or SIGINT, as nextest
/// does at its time limit, removes its containers too. Only SIGKILL leaves one behind, which
/// the label `barras-test=1` finds.
#[derive(Debug)]
pub struct PostgresContainer {
    container: ContainerAsync<ThrowawayPostgres>,
    url: String,
}

impl PostgresContainer {
    /// Starts a container from the image of the `postgres` service of `compose.yaml`, with `fsync`
    /// off, since its data never outlives it, and the `cluster_name` of the local database, by
    /// which `seed` and `reset` know its server for the local one. Waits until it accepts
    /// connections. Its port is published on `127.0.0.1` only, at a port Docker picks. The test
    /// fails when Docker is not running: a test that skipped itself would hide a failure.
    pub async fn start() -> Self {
        Self::start_with(true).await
    }

    /// [`start`](Self::start), without the `cluster_name` of the local database: a server that
    /// stands for one on another machine, such as staging, which `seed` and `reset` must refuse
    /// through whatever port of this machine reaches it.
    pub async fn start_as_another_server() -> Self {
        Self::start_with(false).await
    }

    async fn start_with(local: bool) -> Self {
        let (name, tag) = compose_image();
        let container = ThrowawayPostgres { name, tag, local }
            .with_label(LABEL.0, LABEL.1)
            .with_network(NETWORK)
            .with_host_config_modifier(publish_on_loopback_only)
            .start()
            .await
            .unwrap_or_else(|error| panic!("{}", start_failure("a Postgres container", &error)));
        let url = loopback_url(&container).await;
        Self { container, url }
    }

    /// The URL of the database, with its credentials:
    /// `postgres://barras:barras@127.0.0.1:<port>/barras`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The container's id, for a test that asks Docker about it.
    pub fn id(&self) -> &str {
        self.container.id()
    }

    /// What `psql`, run inside the container as its user on the database `barras`, prints for
    /// `sql`: its rows, one per line, without headers or alignment, trimmed. A test reads and
    /// changes the database this way without a driver of its own, which only the postgres adapter
    /// and the testkit may take. The test fails when psql does.
    pub fn psql(&self, sql: &str) -> String {
        let output = Command::new("docker")
            .args(["exec", self.id(), "psql"])
            .args([format!("--username={USER}"), format!("--dbname={DATABASE}")])
            .args(["--quiet", "--tuples-only", "--no-align", "--command", sql])
            .output()
            .unwrap_or_else(|error| panic!("cannot run psql in the container: {error}"));
        assert!(
            output.status.success(),
            "psql {sql}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap_or_else(|error| {
                panic!("psql {sql} printed something that is not UTF-8: {error}")
            })
            .trim()
            .to_owned()
    }
}

/// A relay from a port of `127.0.0.1` to the Postgres of a [`PostgresContainer`], in a container
/// of its own: what `ssh -L`, `kubectl port-forward` or a cloud SQL proxy makes of a port of this
/// machine, the first hop to a server on another one. Its address passes the local-database
/// guard, whatever server is behind it.
///
/// Dropping it removes its container, as for a [`PostgresContainer`], whose label it carries.
#[derive(Debug)]
pub struct LoopbackRelay {
    container: ContainerAsync<Relay>,
    url: String,
}

impl LoopbackRelay {
    /// Starts a relay to the Postgres of `target`, which it reaches on Docker's bridge network,
    /// and waits until it listens. It runs on the image of `compose.yaml`, so it needs no image of
    /// its own.
    pub async fn start(target: &PostgresContainer) -> Self {
        Self::start_to(&[target]).await
    }

    /// [`start`](Self::start), with a relay that sends each new connection to `first` and
    /// `second` in turn, `first` first: one port of this machine that leads to another server on
    /// each connection, as a proxy that takes turns between two does. What the first connection
    /// learnt of its server says nothing of the server of the next.
    pub async fn start_alternating(first: &PostgresContainer, second: &PostgresContainer) -> Self {
        Self::start_to(&[first, second]).await
    }

    async fn start_to(targets: &[&PostgresContainer]) -> Self {
        let mut addresses = Vec::new();
        for target in targets {
            let address = target
                .container
                .get_bridge_ip_address()
                .await
                .unwrap_or_else(|error| {
                    panic!("the target container has no bridge address: {error}")
                });
            addresses.push(SocketAddr::new(address, PORT.as_u16()).to_string());
        }
        let (name, tag) = compose_image();
        let container = Relay {
            name,
            tag,
            targets: addresses,
        }
        .with_label(LABEL.0, LABEL.1)
        .with_network(NETWORK)
        .with_host_config_modifier(publish_on_loopback_only)
        .start()
        .await
        .unwrap_or_else(|error| panic!("{}", start_failure("a relay container", &error)));
        let url = loopback_url(&container).await;
        Self { container, url }
    }

    /// The URL of the target's database through the relay, with its credentials:
    /// `postgres://barras:barras@127.0.0.1:<port>/barras`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The container's id, for a test that asks Docker about it.
    pub fn id(&self) -> &str {
        self.container.id()
    }
}

/// The image of the `postgres` service of `compose.yaml`, as testcontainers takes it.
fn compose_image() -> (String, String) {
    let compose = fs::read_to_string(repository_file("compose.yaml"))
        .unwrap_or_else(|error| panic!("cannot read compose.yaml: {error}"));
    image_reference(&compose)
}

/// The URL of the database `barras` at the port of `127.0.0.1` where `container` publishes
/// [`PORT`].
async fn loopback_url<I: Image>(container: &ContainerAsync<I>) -> String {
    let port = container
        .get_host_port_ipv4(PORT)
        .await
        .unwrap_or_else(|error| panic!("the container publishes no port for {PORT}: {error}"));
    format!("postgres://{USER}:{PASSWORD}@127.0.0.1:{port}/{DATABASE}")
}

/// The image of `compose.yaml`, run as a throwaway database.
#[derive(Debug)]
struct ThrowawayPostgres {
    name: String,
    tag: String,
    /// Whether its server starts as the local one, with the `cluster_name` of the local database.
    local: bool,
}

impl Image for ThrowawayPostgres {
    fn name(&self) -> &str {
        &self.name
    }

    fn tag(&self) -> &str {
        &self.tag
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        // The image first runs a temporary server to create the database, which prints the line
        // on standard output, then starts the server that stays, which prints it on standard
        // error. Only that second one accepts connections over TCP.
        vec![
            WaitFor::message_on_stderr(READY),
            WaitFor::message_on_stdout(READY),
        ]
    }

    fn env_vars(
        &self,
    ) -> impl IntoIterator<Item = (impl Into<Cow<'_, str>>, impl Into<Cow<'_, str>>)> {
        [
            ("POSTGRES_USER", USER),
            ("POSTGRES_PASSWORD", PASSWORD),
            ("POSTGRES_DB", DATABASE),
        ]
    }

    fn cmd(&self) -> impl IntoIterator<Item = impl Into<Cow<'_, str>>> {
        let mut command = vec!["postgres", "-c", "fsync=off"];
        if self.local {
            command.extend(["-c", LOCAL_CLUSTER_SETTING]);
        }
        command
    }

    fn expose_ports(&self) -> &[ContainerPort] {
        &[PORT]
    }
}

/// The image of `compose.yaml`, run as a relay to `targets` instead of as a database.
#[derive(Debug)]
struct Relay {
    name: String,
    tag: String,
    /// The address and port of each Postgres it relays to, in the order it takes them.
    targets: Vec<String>,
}

impl Image for Relay {
    fn name(&self) -> &str {
        &self.name
    }

    fn tag(&self) -> &str {
        &self.tag
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![WaitFor::message_on_stderr(RELAY_READY)]
    }

    fn entrypoint(&self) -> Option<&str> {
        Some("perl")
    }

    fn cmd(&self) -> impl IntoIterator<Item = impl Into<Cow<'_, str>>> {
        let mut command = vec!["-e", RELAY];
        command.extend(self.targets.iter().map(String::as_str));
        command
    }

    fn expose_ports(&self) -> &[ContainerPort] {
        &[PORT]
    }
}

/// testcontainers publishes every exposed port on all of the host's addresses; this publishes
/// Postgres's on the loopback address alone, at a port Docker picks.
fn publish_on_loopback_only(config: &mut HostConfig) {
    let binding = PortBinding {
        host_ip: Some("127.0.0.1".to_owned()),
        host_port: Some(String::new()),
    };
    config.publish_all_ports = Some(false);
    config.port_bindings = Some(HashMap::from([(PORT.to_string(), Some(vec![binding]))]));
}

/// The image of the `postgres` service of `compose.yaml`, as testcontainers takes it: the name,
/// and the tag with its digest. The service names it on one `image:` line, pinned by tag and
/// digest; the throwaway database of the justfile reads the same service's image. Read by the
/// service, so that another service beside it, with an image of its own, changes nothing.
fn image_reference(compose: &str) -> (String, String) {
    // Each line that holds something, with its indentation, which is how YAML nests a key.
    let lines: Vec<Line<'_>> = compose
        .lines()
        .filter_map(|line| {
            let text = line.trim_start();
            (!text.is_empty() && !text.starts_with('#')).then(|| (line.len() - text.len(), text))
        })
        .collect();
    let service = inside(&lines, "services", 0).and_then(|services| {
        let level = services.first()?.0;
        inside(services, "postgres", level)
    });
    let Some(service) = service else {
        panic!("compose.yaml must have a `postgres` service under `services:`");
    };
    // The service's own keys, not those nested deeper inside one of them.
    let level = service.first().map_or(0, |&(indent, _)| indent);
    let references: Vec<&str> = service
        .iter()
        .filter(|&&(indent, _)| indent == level)
        .filter_map(|&(_, text)| {
            let value = text.strip_prefix("image:")?;
            let word = value.split_whitespace().next().unwrap_or_default();
            Some(word.trim_matches(['"', '\'']))
        })
        .collect();
    let [reference] = references[..] else {
        panic!(
            "the postgres service of compose.yaml must name its image on one `image:` line; it \
             has {}",
            references.len()
        );
    };
    let pinned = reference.split_once('@').and_then(|(named, digest)| {
        // A `:` before the last `/` is a registry's port, not a tag.
        let tag_start = named
            .rfind(':')
            .filter(|&colon| !named[colon..].contains('/'))?;
        let tag = &named[tag_start + 1..];
        (!tag.is_empty() && !digest.is_empty())
            .then(|| (named[..tag_start].to_owned(), format!("{tag}@{digest}")))
    });
    pinned.unwrap_or_else(|| {
        panic!(
            "the image of compose.yaml must be pinned by tag and digest, as in name:tag@sha256:…"
        )
    })
}

/// A line of a compose file that holds something: its indentation, and its text after it.
type Line<'a> = (usize, &'a str);

/// The lines of the mapping that `key:`, written at `indent`, opens among `lines`: those after
/// it that are indented deeper, up to the first that is not.
fn inside<'l, 'a>(lines: &'l [Line<'a>], key: &str, indent: usize) -> Option<&'l [Line<'a>]> {
    let opens = |text: &str| {
        text.strip_prefix(key)
            .and_then(|rest| rest.strip_prefix(':'))
            .map(str::trim_start)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('#'))
    };
    let start = 1 + lines
        .iter()
        .position(|&(at, text)| at == indent && opens(text))?;
    let length = lines[start..]
        .iter()
        .take_while(|&&(at, _)| at > indent)
        .count();
    Some(&lines[start..start + length])
}

/// What a test that cannot start `container` fails with. testcontainers reports a Docker that is
/// not running in more than one way, so the sentence stands after any failure.
fn start_failure(container: &str, error: &TestcontainersError) -> String {
    format!("cannot start {container}: {error}. The int_ and e2e_ tests need Docker running.")
}

#[cfg(test)]
mod tests {
    use testcontainers::bollard::errors::Error as DockerError;
    use testcontainers::core::error::ClientError;

    use super::*;

    const DIGEST: &str = "sha256:2ea8630df176ad090ec92136dee850397c5af8781e4bcf2e4162593691c61bbc";

    fn compose_with(image_line: &str) -> String {
        format!("services:\n  postgres:\n    # image: not this one\n{image_line}\n    ports:\n")
    }

    #[test]
    fn the_image_is_the_one_compose_yaml_names_with_its_tag_and_digest() {
        let compose = fs::read_to_string(repository_file("compose.yaml")).unwrap();

        let (name, tag) = image_reference(&compose);

        assert_eq!(name, "postgres");
        assert!(tag.starts_with("18."), "{tag}");
        assert!(tag.contains("@sha256:"), "{tag}");
    }

    #[test]
    fn a_reference_splits_at_the_tag_after_the_last_slash() {
        let compose = compose_with(&format!(
            "    image: registry.example:5000/library/postgres:18.7-trixie@{DIGEST}"
        ));

        assert_eq!(
            image_reference(&compose),
            (
                "registry.example:5000/library/postgres".to_owned(),
                format!("18.7-trixie@{DIGEST}")
            )
        );
    }

    #[test]
    fn a_quoted_reference_followed_by_a_comment_is_read_as_written() {
        let compose = compose_with(&format!(
            "    image: \"postgres:18.7-trixie@{DIGEST}\" # pinned"
        ));

        assert_eq!(
            image_reference(&compose),
            ("postgres".to_owned(), format!("18.7-trixie@{DIGEST}"))
        );
    }

    /// A Postgres service and a Redis one, as BAR-5 adds it, each with its image, in either
    /// order, and other keys before, between and after them.
    fn compose_with_redis(redis_first: bool) -> String {
        let postgres = format!(
            "  postgres: # the local database\n    image: postgres:18.7-trixie@{DIGEST}\n    \
             ports:\n      - \"127.0.0.1:5433:5432\"\n    healthcheck:\n      interval: 1s\n"
        );
        let redis = format!(
            "  redis:\n    image: redis:8.4-trixie@{DIGEST}\n    ports:\n      - \
             \"127.0.0.1:6380:6379\"\n"
        );
        let (first, second) = if redis_first {
            (redis, postgres)
        } else {
            (postgres, redis)
        };
        format!(
            "name: barras\nservices:\n{first}\n  # another service\n{second}volumes:\n  data:\n"
        )
    }

    #[test]
    fn the_image_is_the_postgres_service_s_beside_another_service() {
        for redis_first in [true, false] {
            let compose = compose_with_redis(redis_first);

            assert_eq!(
                image_reference(&compose),
                ("postgres".to_owned(), format!("18.7-trixie@{DIGEST}")),
                "{compose}"
            );
        }
    }

    #[test]
    fn an_image_line_outside_the_service_s_own_keys_is_not_read() {
        // Deeper inside the service, and in a top-level key after `services:`.
        let compose = format!(
            "services:\n  postgres:\n    labels:\n      image: not-this:1@{DIGEST}\n    image: \
             postgres:18.7-trixie@{DIGEST}\nx-other:\n  image: nor-this:1@{DIGEST}\n"
        );

        assert_eq!(
            image_reference(&compose),
            ("postgres".to_owned(), format!("18.7-trixie@{DIGEST}"))
        );
    }

    #[test]
    #[should_panic(expected = "compose.yaml must have a `postgres` service")]
    fn a_compose_file_without_a_postgres_service_fails_the_test() {
        image_reference(&format!(
            "services:\n  redis:\n    image: redis:8.4-trixie@{DIGEST}\n"
        ));
    }

    #[test]
    #[should_panic(expected = "compose.yaml must have a `postgres` service")]
    fn a_postgres_key_outside_services_is_not_the_service() {
        image_reference(&format!(
            "x-services:\n  postgres:\n    image: postgres:18.7-trixie@{DIGEST}\n"
        ));
    }

    #[test]
    #[should_panic(expected = "must name its image on one `image:` line; it has 0")]
    fn a_compose_file_without_an_image_fails_the_test() {
        image_reference(&compose_with("    build: ."));
    }

    #[test]
    #[should_panic(expected = "must name its image on one `image:` line; it has 2")]
    fn a_compose_file_with_two_images_fails_the_test() {
        image_reference(&compose_with(&format!(
            "    image: postgres:18.7-trixie@{DIGEST}\n    image: postgres:17@{DIGEST}"
        )));
    }

    #[test]
    #[should_panic(expected = "pinned by tag and digest")]
    fn an_image_without_a_digest_fails_the_test() {
        image_reference(&compose_with("    image: postgres:18.7-trixie"));
    }

    #[test]
    #[should_panic(expected = "pinned by tag and digest")]
    fn an_image_without_a_tag_fails_the_test() {
        image_reference(&compose_with(&format!(
            "    image: registry.example:5000/postgres@{DIGEST}"
        )));
    }

    #[test]
    fn without_docker_the_failure_says_the_int_and_e2e_tests_need_it() {
        // What testcontainers answers when no Docker socket exists (plan, appendix A.7).
        let error = TestcontainersError::Client(ClientError::Init(
            DockerError::SocketNotFoundError("/var/run/docker.sock".to_owned()),
        ));

        let failure = start_failure("a Postgres container", &error);

        assert!(
            failure.starts_with("cannot start a Postgres container: "),
            "{failure}"
        );
        assert!(failure.contains("/var/run/docker.sock"), "{failure}");
        assert!(
            failure.ends_with("The int_ and e2e_ tests need Docker running."),
            "{failure}"
        );
    }
}
