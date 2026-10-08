//! Names only: never retain the channel's messages, members or topic.
use super::notifier::safe_text;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::Instant;

const CACHE_CAPACITY: usize = 128;
const CACHE_TTL: Duration = Duration::from_secs(300);
const RETRY_DELAY: Duration = Duration::from_secs(30);
const NAME_LENGTH: usize = 32;

struct Entry<T> {
    value: Option<T>,
    expires: Instant,
}
pub(super) struct MetadataCache<T> {
    entries: HashMap<String, Entry<T>>,
    retry_at: Option<Instant>,
}
impl<T: Clone> MetadataCache<T> {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            retry_at: None,
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.retry_at = None;
    }
    pub fn get(&self, id: &str) -> Option<Option<T>> {
        self.entries
            .get(id)
            .filter(|entry| entry.expires > Instant::now())
            .map(|entry| entry.value.clone())
    }
    pub fn can_request(&self) -> bool {
        self.retry_at
            .is_none_or(|deadline| deadline <= Instant::now())
    }
    pub fn insert(&mut self, id: String, value: Option<T>) {
        if self.entries.len() >= CACHE_CAPACITY && !self.entries.contains_key(&id) {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.expires)
                .map(|(key, _)| key.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        let ttl = if value.is_some() {
            CACHE_TTL
        } else {
            RETRY_DELAY
        };
        if value.is_none() {
            // Unsupported RPC commands must not stall every subsequent notification.
            self.retry_at = Some(Instant::now() + RETRY_DELAY);
        }
        self.entries.insert(
            id,
            Entry {
                value,
                expires: Instant::now() + ttl,
            },
        );
    }
}

fn first_string(values: &[Option<&Value>]) -> Option<String> {
    values.iter().find_map(|value| {
        value
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned)
    })
}
fn name(values: &[Option<&Value>]) -> Option<String> {
    first_string(values)
        .map(|text| {
            safe_text(&text, NAME_LENGTH)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|text| !text.is_empty())
}
fn id(values: &[Option<&Value>]) -> Option<String> {
    first_string(values).filter(|id| {
        id.len() <= 20
            && !id.starts_with('0')
            && id.bytes().all(|c| c.is_ascii_digit())
            && id.parse::<u64>().is_ok()
    })
}

#[derive(Clone, Default)]
pub(super) struct NotificationContext {
    pub guild_id: Option<String>,
    pub guild_name: Option<String>,
    pub channel_name: Option<String>,
    pub is_dm: bool,
}
impl NotificationContext {
    pub fn from_event(data: &Value, message: &Value) -> Self {
        let channel = data
            .get("channel")
            .or_else(|| message.get("channel"))
            .unwrap_or(&Value::Null);
        let guild = data
            .get("guild")
            .or_else(|| message.get("guild"))
            .unwrap_or(&Value::Null);
        Self {
            guild_id: id(&[
                data.get("guild_id"),
                message.get("guild_id"),
                channel.get("guild_id"),
                guild.get("id"),
            ]),
            guild_name: name(&[
                data.get("guild_name"),
                message.get("guild_name"),
                guild.get("name"),
            ]),
            channel_name: name(&[
                data.get("channel_name"),
                message.get("channel_name"),
                channel.get("name"),
            ]),
            is_dm: matches!(
                channel
                    .get("type")
                    .or_else(|| data.get("channel_type"))
                    .and_then(Value::as_u64),
                Some(1 | 3)
            ),
        }
    }
    pub fn from_channel(channel: &Value) -> Self {
        Self {
            guild_id: id(&[channel.get("guild_id")]),
            channel_name: name(&[channel.get("name")]),
            is_dm: matches!(channel.get("type").and_then(Value::as_u64), Some(1 | 3)),
            ..Self::default()
        }
    }
    pub fn guild_name(guild: &Value) -> Option<String> {
        name(&[guild.get("name")])
    }
    pub fn merge_channel(&mut self, channel: Self) {
        self.is_dm |= channel.is_dm;
        if self.guild_id.is_none() {
            self.guild_id = channel.guild_id;
        }
        if self.channel_name.is_none() {
            self.channel_name = channel.channel_name;
        }
    }
    pub fn line(&self) -> Option<String> {
        if self.is_dm || (self.guild_id.is_none() && self.guild_name.is_none()) {
            return None;
        }
        Some(format!(
            "{} / #{}",
            self.guild_name.as_deref().unwrap_or("サーバー"),
            self.channel_name.as_deref().unwrap_or("チャンネル")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn cache_is_bounded_expires_and_backs_off_failed_requests() {
        let mut cache = MetadataCache::new();
        for index in 0..200 {
            cache.insert(index.to_string(), Some("name".to_owned()));
        }
        assert_eq!(cache.entries.len(), CACHE_CAPACITY);
        cache.insert("failure".into(), None);
        assert!(!cache.can_request());
        assert_eq!(cache.get("failure"), Some(None));
        tokio::time::advance(Duration::from_secs(31)).await;
        assert!(cache.can_request());
        assert_eq!(cache.get("failure"), None);
        assert_eq!(cache.get("199"), Some(Some("name".into())));
        tokio::time::advance(Duration::from_secs(300)).await;
        assert_eq!(cache.get("199"), None);
        cache.clear();
        assert!(cache.entries.is_empty());
    }
    #[test]
    fn context_sanitizes_names_and_keeps_dm_and_group_dm_unchanged() {
        let context = NotificationContext::from_event(
            &serde_json::json!({
                "guild": {"id":"10", "name":"<b>Server</b>\nName"},
                "channel": {"name":"general\tchat", "type":0}
            }),
            &Value::Null,
        );
        assert_eq!(
            context.line().unwrap(),
            "＜b＞Server＜/b＞ Name / #general chat"
        );
        for kind in [1, 3] {
            let mut dm = context.clone();
            dm.merge_channel(NotificationContext::from_channel(
                &serde_json::json!({"type":kind,"name":"DM"}),
            ));
            assert!(dm.line().is_none());
        }
        assert!(NotificationContext::default().line().is_none());
    }
}
