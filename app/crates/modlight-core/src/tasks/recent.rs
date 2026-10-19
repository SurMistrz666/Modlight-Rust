//! The last few finished uploads (`RecentTask`, `RecentTaskManager`): what the tray's
//! "Recent items" offers, kept in `ApplicationConfig.RecentTasks` when asked to.

use std::collections::VecDeque;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::dotnet;

/// `RecentTask`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RecentTask {
    #[serde(deserialize_with = "dotnet::nullable_string::deserialize")]
    pub file_path: String,
    #[serde(rename = "URL", deserialize_with = "dotnet::nullable_string::deserialize")]
    pub url: String,
    #[serde(rename = "ThumbnailURL", deserialize_with = "dotnet::nullable_string::deserialize")]
    pub thumbnail_url: String,
    #[serde(rename = "DeletionURL", deserialize_with = "dotnet::nullable_string::deserialize")]
    pub deletion_url: String,
    #[serde(rename = "ShortenedURL", deserialize_with = "dotnet::nullable_string::deserialize")]
    pub shortened_url: String,
    #[serde(with = "dotnet::utc")]
    pub time: DateTime<Utc>,
}

impl Default for RecentTask {
    fn default() -> Self {
        RecentTask {
            file_path: String::new(),
            url: String::new(),
            thumbnail_url: String::new(),
            deletion_url: String::new(),
            shortened_url: String::new(),
            time: Utc::now(),
        }
    }
}

impl RecentTask {
    /// The file's name, or the last part of the URL when there is no file.
    pub fn file_name(&self) -> String {
        let text = if !self.file_path.is_empty() {
            &self.file_path
        } else {
            &self.url
        };
        Path::new(text.trim_end_matches('/'))
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The shortened URL, else the URL, else the file path.
    pub fn text(&self) -> &str {
        if !self.shortened_url.is_empty() {
            &self.shortened_url
        } else if !self.url.is_empty() {
            &self.url
        } else {
            &self.file_path
        }
    }
}

/// `RecentTaskManager`: a bounded queue, oldest out first.
#[derive(Clone, Debug)]
pub struct RecentTaskManager {
    max_count: usize,
    pub tasks: VecDeque<RecentTask>,
}

impl Default for RecentTaskManager {
    fn default() -> Self {
        RecentTaskManager {
            max_count: 10,
            tasks: VecDeque::new(),
        }
    }
}

impl RecentTaskManager {
    pub fn max_count(&self) -> usize {
        self.max_count
    }

    /// Clamped to 1..=100, and the list cut down to it.
    pub fn set_max_count(&mut self, count: i64) {
        self.max_count = count.clamp(1, 100) as usize;
        while self.tasks.len() > self.max_count {
            self.tasks.pop_front();
        }
    }

    /// `InitItems`: what the settings kept, up to the limit.
    pub fn init(&mut self, max_count: i64, saved: Option<Vec<RecentTask>>) {
        self.set_max_count(max_count);
        if let Some(saved) = saved {
            self.tasks = saved.into_iter().take(self.max_count).collect();
        }
    }

    pub fn add(&mut self, task: RecentTask) {
        while self.tasks.len() >= self.max_count {
            self.tasks.pop_front();
        }
        self.tasks.push_back(task);
    }

    pub fn clear(&mut self) {
        self.tasks.clear();
    }

    pub fn to_vec(&self) -> Vec<RecentTask> {
        self.tasks.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_bounded() {
        let mut recent = RecentTaskManager::default();
        recent.set_max_count(2);
        for i in 0..3 {
            recent.add(RecentTask {
                url: format!("https://h/{i}"),
                ..RecentTask::default()
            });
        }
        assert_eq!(recent.tasks.len(), 2);
        assert_eq!(recent.tasks[0].url, "https://h/1");
        recent.set_max_count(0);
        assert_eq!(recent.max_count(), 1);
        assert_eq!(recent.tasks.len(), 1);
    }

    #[test]
    fn names_and_text() {
        let task = RecentTask {
            url: "https://files.example.com/alex/abc".into(),
            ..RecentTask::default()
        };
        assert_eq!(task.file_name(), "abc");
        assert_eq!(task.text(), "https://files.example.com/alex/abc");
        let json = serde_json::to_value(&task).unwrap();
        assert!(json.get("URL").is_some() && json.get("ShortenedURL").is_some());
    }
}
