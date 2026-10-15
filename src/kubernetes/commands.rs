use super::model::{Resource, ResourceKind};
use crate::i18n::t;

const STATUS: &str = "__SHELLRS_K8S_EXIT__:";
pub(super) const LOG_BYTES: usize = 512 * 1024;

/// Quote every argument, including contexts from an untrusted kubeconfig.
pub(super) fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

pub(super) fn kubectl(context: &str) -> String {
    format!("kubectl --context={}", quote(context))
}

/// SSH's shared exec API returns stdout only. Merge stderr and frame the
/// exit status so Forbidden, missing kubectl, and truncation stay visible.
pub(super) fn remote(command: &str) -> String {
    let command = command.replacen("kubectl ", "kubectl --request-timeout=8s ", 1);
    let script = format!(
        "export LC_ALL=C PATH=\"$PATH:/usr/local/bin:/opt/homebrew/bin\"; {{\n{command}\n}} 2>&1; code=$?; printf '\\n{STATUS}%s\\n' \"$code\""
    );
    // The login shell may be fish or csh. Encode the script so the outer
    // single-quoted argument has neither nested quotes nor history `!`.
    let encoded = script
        .bytes()
        .map(|byte| format!("\\0{byte:03o}"))
        .collect::<String>();
    format!("sh -c 'printf \"%b\" \"{encoded}\" | sh'")
}

/// sshd hands a command to the login shell as one argument, which Linux caps
/// at 128 KiB (MAX_ARG_STRLEN). `remote` sends every byte of the script as
/// five (`\0NNN`), so a write carries about 25 KB of JSON.
pub(super) const COMMAND_LIMIT: usize = 127 * 1024;

/// The executable precedes the payload, so request-timeout insertion cannot
/// modify a ConfigMap value which happens to contain the word "kubectl".
pub(super) fn remote_stdin(command: &str, value: &serde_json::Value) -> String {
    let body = serde_json::to_string(value).expect("JSON values serialize");
    remote(&format!(
        "{command} <<'__SHELLRS_K8S_INPUT__'\n{body}\n__SHELLRS_K8S_INPUT__"
    ))
}

pub(super) const WIRE_LIMIT: usize = 1 << 20;
pub(super) const DECODED_LIMIT: usize = 16 << 20;
pub(super) const COMPRESSED: &str = "__SHELLRS_K8S_GZIP__\n";

/// Compress only structured reads. Plain framing remains a portable fallback.
pub(super) fn remote_data(command: &str) -> String {
    let command = command.replacen("kubectl ", "kubectl --request-timeout=8s ", 1);
    let framed = format!("{{ {command}; }} 2>&1; code=$?; printf '\\n{STATUS}%s\\n' \"$code\"");
    let script = format!(
        "export LC_ALL=C PATH=\"$PATH:/usr/local/bin:/opt/homebrew/bin\"; if command -v gzip >/dev/null 2>&1 && command -v base64 >/dev/null 2>&1; then printf '{COMPRESSED}'; {{ {framed}; }} | gzip -1 -c | base64; else {framed}; fi"
    );
    let encoded = script
        .bytes()
        .map(|byte| format!("\\0{byte:03o}"))
        .collect::<String>();
    format!("sh -c 'printf \"%b\" \"{encoded}\" | sh'")
}

#[derive(Debug)]
pub(super) enum ReadError {
    TooLarge,
    Message(String),
}
impl ReadError {
    pub fn message(self) -> String {
        match self {
            Self::TooLarge => t!("k8s.query_too_large").to_string(),
            Self::Message(message) => message,
        }
    }
}
fn decoded(output: &str) -> Result<String, ReadError> {
    use base64::Engine as _;
    use std::io::Read as _;
    let Some(encoded) = output.strip_prefix(COMPRESSED) else {
        return Ok(output.to_owned());
    };
    let encoded = encoded
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    let incomplete = || ReadError::Message(t!("k8s.incomplete").to_string());
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| incomplete())?;
    let mut bytes_out = Vec::new();
    flate2::read::GzDecoder::new(bytes.as_slice())
        .take((DECODED_LIMIT + 1) as u64)
        .read_to_end(&mut bytes_out)
        .map_err(|_| incomplete())?;
    if bytes_out.len() > DECODED_LIMIT {
        return Err(ReadError::TooLarge);
    }
    String::from_utf8(bytes_out).map_err(|_| incomplete())
}
pub(super) fn answer_bounded(output: &str) -> Result<String, ReadError> {
    let output = decoded(output)?;
    let (body, status) = output
        .rsplit_once(&format!("\n{STATUS}"))
        .ok_or_else(|| ReadError::Message(t!("k8s.incomplete").to_string()))?;
    match status.trim() {
        "0" => Ok(body.to_owned()),
        code => Err(ReadError::Message(format!("{} (exit {code})", body.trim()))),
    }
}
pub(super) fn answer(output: &str) -> Result<String, String> {
    answer_bounded(output).map_err(ReadError::message)
}

pub(super) fn contexts_command() -> String {
    remote(
        "kubectl config current-context && printf '\\n__SHELLRS_CONTEXTS__\\n' && kubectl config get-contexts -o name",
    )
}

pub(super) fn contexts(output: &str) -> Result<(String, Vec<String>), String> {
    let output = answer(output)?;
    let (current, rest) = output
        .split_once("\n__SHELLRS_CONTEXTS__\n")
        .ok_or_else(|| t!("k8s.incomplete").to_string())?;
    let current = current.trim().to_owned();
    let mut names = rest
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    if current.is_empty() || !names.contains(&current) {
        return Err(t!("k8s.no_context").to_string());
    }
    Ok((current, names))
}

pub(super) fn namespaces(context: &str) -> String {
    list(context, None, ResourceKind::Namespace, None)
}

pub(super) const PAGE_SIZE: usize = 100;

#[derive(Clone, Copy)]
pub(super) struct ListQuery<'a> {
    context: &'a str,
    namespace: Option<&'a str>,
    kind: ResourceKind,
    selector: Option<&'a str>,
    field_selector: Option<&'a str>,
}
impl<'a> ListQuery<'a> {
    pub fn new(context: &'a str, namespace: Option<&'a str>, kind: ResourceKind) -> Self {
        Self {
            context,
            namespace,
            kind,
            selector: None,
            field_selector: None,
        }
    }
    pub fn selector(mut self, selector: Option<&'a str>) -> Self {
        self.selector = selector;
        self
    }
    pub fn field_selector(mut self, selector: Option<&'a str>) -> Self {
        self.field_selector = selector;
        self
    }
    pub fn kind(&self) -> ResourceKind {
        self.kind
    }
    pub fn paginated(&self) -> bool {
        !matches!(self.kind, ResourceKind::Secret | ResourceKind::ConfigMap)
    }
    pub fn command(&self, continuation: &str, limit: usize) -> String {
        if !self.paginated() {
            let scope = self
                .namespace
                .map(|ns| format!("--namespace={}", quote(ns)))
                .unwrap_or_else(|| "--all-namespaces".into());
            return remote_data(&format!(
                "{} get {} {scope} -o jsonpath-as-json='{{.items[*].metadata}}' --chunk-size=100",
                kubectl(self.context),
                self.kind.resource()
            ));
        }
        let group = match self.kind {
            ResourceKind::Deployment
            | ResourceKind::StatefulSet
            | ResourceKind::DaemonSet
            | ResourceKind::ReplicaSet => "/apis/apps/v1",
            ResourceKind::Ingress => "/apis/networking.k8s.io/v1",
            ResourceKind::Job | ResourceKind::CronJob => "/apis/batch/v1",
            ResourceKind::StorageClass => "/apis/storage.k8s.io/v1",
            ResourceKind::HorizontalPodAutoscaler => "/apis/autoscaling/v2",
            ResourceKind::PodDisruptionBudget => "/apis/policy/v1",
            ResourceKind::EndpointSlice => "/apis/discovery.k8s.io/v1",
            _ => "/api/v1",
        };
        let escape = |value: &str| {
            percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
        };
        let scope = self
            .namespace
            .filter(|_| self.kind.namespaced())
            .map(|ns| format!("/namespaces/{}", escape(ns)))
            .unwrap_or_default();
        let mut path = format!(
            "{group}{scope}/{}?limit={}",
            self.kind.resource(),
            limit.clamp(1, PAGE_SIZE)
        );
        for (name, value) in [
            (
                "continue",
                (!continuation.is_empty()).then_some(continuation),
            ),
            ("labelSelector", self.selector),
            ("fieldSelector", self.field_selector),
        ] {
            if let Some(value) = value {
                path.push_str(&format!("&{name}={}", escape(value)));
            }
        }
        remote_data(&format!(
            "{} get --raw={}",
            kubectl(self.context),
            quote(&path)
        ))
    }
}

pub(super) fn list(
    context: &str,
    namespace: Option<&str>,
    kind: ResourceKind,
    selector: Option<&str>,
) -> String {
    ListQuery::new(context, namespace, kind)
        .selector(selector)
        .command("", PAGE_SIZE)
}

pub(super) fn detail(context: &str, resource: &Resource) -> String {
    if resource.kind == ResourceKind::Secret {
        return remote_data(&generated(
            context,
            resource,
            "get",
            Some("-o jsonpath-as-json='{.metadata}'"),
        ));
    }
    remote_data(&generated(context, resource, "get", Some("-o json")))
}

fn generated(context: &str, resource: &Resource, verb: &str, extra: Option<&str>) -> String {
    let scope = if resource.kind.namespaced() {
        format!("--namespace={}", quote(&resource.namespace))
    } else {
        String::new()
    };
    format!(
        "{} {verb} {} {} {} {}",
        kubectl(context),
        resource.kind.resource(),
        quote(&resource.name),
        scope,
        extra.unwrap_or("")
    )
    .trim_end()
    .to_owned()
}

#[derive(Clone, Debug)]
pub(super) struct LogOptions {
    pub container: String,
    pub tail: u32,
    pub since: String,
    pub previous: bool,
}

impl Default for LogOptions {
    fn default() -> Self {
        Self {
            container: String::new(),
            tail: 1000,
            since: "1h".to_owned(),
            previous: false,
        }
    }
}

pub(super) fn logs(
    context: &str,
    resource: &Resource,
    options: &LogOptions,
    follow: bool,
) -> Option<String> {
    if resource.kind != ResourceKind::Pod || options.container.is_empty() {
        return None;
    }
    let mut command = format!(
        "{} logs {} --namespace={} --container={} --tail={} --timestamps=true",
        kubectl(context),
        quote(&resource.name),
        quote(&resource.namespace),
        quote(&options.container),
        options.tail.clamp(1, 10000)
    );
    if !follow {
        command.push_str(&format!(" --limit-bytes={LOG_BYTES}"));
    }
    if !options.since.is_empty() {
        command.push_str(&format!(" --since={}", quote(&options.since)));
    }
    if options.previous {
        command.push_str(" --previous=true");
    }
    if follow {
        command.push_str(" --follow=true");
    }
    Some(command)
}

pub(super) fn suggestions(
    context: &str,
    resource: &Resource,
    options: &LogOptions,
) -> Vec<(&'static str, String)> {
    let mut commands = vec![
        (
            "k8s.command.describe",
            generated(context, resource, "describe", None),
        ),
        (
            "k8s.command.yaml",
            generated(context, resource, "get", Some("-o yaml")),
        ),
    ];
    if let Some(logs) = logs(context, resource, options, false) {
        commands.push(("k8s.command.logs", logs));
        commands.push((
            "k8s.command.follow",
            logs_command_follow(context, resource, options),
        ));
        commands.push((
            "k8s.command.exec",
            format!(
                "{} exec -it {} --namespace={} --container={} -- /bin/sh",
                kubectl(context),
                quote(&resource.name),
                quote(&resource.namespace),
                quote(&options.container)
            ),
        ));
    }
    commands
}

fn logs_command_follow(context: &str, resource: &Resource, options: &LogOptions) -> String {
    logs(context, resource, options, true).unwrap_or_default()
}
