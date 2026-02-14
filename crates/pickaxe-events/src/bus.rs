use std::collections::HashMap;

/// Event priority levels (executed in order: Lowest first, Monitor last).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Lowest = 0,
    Low = 1,
    Normal = 2,
    High = 3,
    Highest = 4,
    Monitor = 5,
}

impl Priority {
    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "LOWEST" => Priority::Lowest,
            "LOW" => Priority::Low,
            "NORMAL" => Priority::Normal,
            "HIGH" => Priority::High,
            "HIGHEST" => Priority::Highest,
            "MONITOR" => Priority::Monitor,
            _ => Priority::Normal,
        }
    }
}

/// Result of handling an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventResult {
    Continue,
    Cancel,
}

/// A registered listener with metadata (the actual callback is stored in Lua registry).
#[derive(Debug, Clone)]
pub struct ListenerEntry {
    pub mod_id: String,
    pub priority: Priority,
    /// Unique ID for this listener, used to retrieve the Lua callback.
    pub listener_id: u64,
}

/// The event bus: maps event names to sorted listener lists.
pub struct EventBus {
    listeners: HashMap<String, Vec<ListenerEntry>>,
    next_listener_id: u64,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            listeners: HashMap::new(),
            next_listener_id: 1,
        }
    }

    /// Register a listener for an event. Returns the listener_id.
    pub fn register(&mut self, event_name: &str, mod_id: &str, priority: Priority) -> u64 {
        let listener_id = self.next_listener_id;
        self.next_listener_id += 1;

        let entry = ListenerEntry {
            mod_id: mod_id.to_string(),
            priority,
            listener_id,
        };

        let list = self.listeners.entry(event_name.to_string()).or_default();
        list.push(entry);
        list.sort_by_key(|e| e.priority);

        listener_id
    }

    /// Get all listeners for an event, sorted by priority.
    pub fn get_listeners(&self, event_name: &str) -> &[ListenerEntry] {
        self.listeners
            .get(event_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get the number of registered events.
    pub fn event_count(&self) -> usize {
        self.listeners.len()
    }

    /// Get total listener count across all events.
    pub fn listener_count(&self) -> usize {
        self.listeners.values().map(|v| v.len()).sum()
    }
}

/// Registry for function overrides (secondary mod API).
pub struct OverrideRegistry {
    overrides: HashMap<String, OverrideEntry>,
}

pub struct OverrideEntry {
    pub mod_id: String,
    pub listener_id: u64,
    pub original_listener_id: Option<u64>,
}

impl OverrideRegistry {
    pub fn new() -> Self {
        Self {
            overrides: HashMap::new(),
        }
    }

    pub fn register(&mut self, function_name: &str, entry: OverrideEntry) {
        if self.overrides.contains_key(function_name) {
            tracing::warn!(
                "Override for '{}' replaced by mod '{}'",
                function_name,
                entry.mod_id
            );
        }
        self.overrides.insert(function_name.to_string(), entry);
    }

    pub fn get(&self, function_name: &str) -> Option<&OverrideEntry> {
        self.overrides.get(function_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Lowest < Priority::Low);
        assert!(Priority::Low < Priority::Normal);
        assert!(Priority::Normal < Priority::High);
        assert!(Priority::High < Priority::Highest);
        assert!(Priority::Highest < Priority::Monitor);
    }

    #[test]
    fn test_event_bus_registration() {
        let mut bus = EventBus::new();
        bus.register("player_join", "vanilla", Priority::Normal);
        bus.register("player_join", "my-mod", Priority::High);
        bus.register("player_join", "early-mod", Priority::Lowest);

        let listeners = bus.get_listeners("player_join");
        assert_eq!(listeners.len(), 3);
        assert_eq!(listeners[0].mod_id, "early-mod");
        assert_eq!(listeners[1].mod_id, "vanilla");
        assert_eq!(listeners[2].mod_id, "my-mod");
    }

    #[test]
    fn test_listener_ids_are_unique() {
        let mut bus = EventBus::new();
        let id1 = bus.register("test", "mod1", Priority::Normal);
        let id2 = bus.register("test", "mod2", Priority::Normal);
        assert_ne!(id1, id2);
    }
}
