//! API pages cross SSH independently; parsing and decompression stay off the UI thread.
use super::{
    commands::{self, ListQuery},
    model,
    panel::request,
};
use crate::{i18n::t, terminal::ExecTarget};
use gpui_kit::AsyncApp;
use serde_json::Value;
use std::collections::HashSet;

pub(super) struct Page {
    pub items: Vec<Value>,
    pub next: String,
    pub version: String,
    pub bytes: usize,
}
pub(super) fn page(output: &str, kind: model::ResourceKind) -> Result<Page, commands::ReadError> {
    let output = commands::answer_bounded(output)?;
    let bytes = output.len();
    let mut value = model::list_value(&output, kind).map_err(commands::ReadError::Message)?;
    let next = match &value["metadata"]["continue"] {
        Value::Null => String::new(),
        Value::String(next) => next.clone(),
        _ => {
            return Err(commands::ReadError::Message(
                t!("k8s.incomplete").to_string(),
            ));
        }
    };
    let version = value["metadata"]["resourceVersion"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    let Value::Array(mut items) = value["items"].take() else {
        return Err(commands::ReadError::Message(
            t!("k8s.incomplete").to_string(),
        ));
    };
    for item in &mut items {
        if let Some(metadata) = item["metadata"].as_object_mut() {
            metadata.remove("managedFields");
            if let Some(annotations) = metadata
                .get_mut("annotations")
                .and_then(Value::as_object_mut)
            {
                annotations.remove("kubectl.kubernetes.io/last-applied-configuration");
            }
        }
    }
    Ok(Page {
        items,
        next,
        version,
        bytes,
    })
}

pub(super) fn smaller_limit(query: ListQuery<'_>, output: &str, limit: usize) -> Option<usize> {
    (query.paginated() && output.len() >= commands::WIRE_LIMIT && limit > 1)
        .then(|| (limit / 2).max(1))
}

pub(super) async fn fetch(
    target: &ExecTarget,
    query: ListQuery<'_>,
    first: Option<String>,
    cx: &mut AsyncApp,
    mut on_page: impl FnMut(&[Value], &mut AsyncApp) -> bool,
) -> Result<Vec<Value>, String> {
    let mut first = first;
    let mut next = String::new();
    let mut seen = HashSet::new();
    let mut version = String::new();
    let mut limit = commands::PAGE_SIZE;
    let mut total_bytes = 0usize;
    let mut items = Vec::new();
    loop {
        let output = match first.take() {
            Some(output) => output,
            None => request(target, query.command(&next, limit), cx).await?,
        };
        let retry_limit = smaller_limit(query, &output, limit);
        let kind = query.kind();
        let page = cx
            .background_executor()
            .spawn(async move { page(&output, kind) })
            .await;
        let page = match page {
            Ok(page) => page,
            Err(commands::ReadError::TooLarge) if query.paginated() && limit > 1 => {
                limit = (limit / 2).max(1);
                continue;
            }
            Err(_) if retry_limit.is_some() => {
                limit = retry_limit.unwrap();
                continue;
            }
            Err(why) => return Err(why.message()),
        };
        if !version.is_empty() && !page.version.is_empty() && version != page.version {
            return Err(t!("k8s.incomplete").to_string());
        }
        if version.is_empty() {
            version = page.version;
        }
        total_bytes += page.bytes;
        if total_bytes > 64 << 20 || items.len() + page.items.len() > 100_000 {
            return Err(t!("k8s.query_too_large").to_string());
        }
        if !page.next.is_empty() && (!query.paginated() || !seen.insert(page.next.clone())) {
            return Err(t!("k8s.incomplete").to_string());
        }
        if !on_page(&page.items, cx) {
            return Err(t!("tools.not_connected").to_string());
        }
        items.extend(page.items);
        if page.next.is_empty() {
            return Ok(items);
        }
        next = page.next;
    }
}
