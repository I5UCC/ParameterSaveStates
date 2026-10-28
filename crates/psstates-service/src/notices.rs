//! User-facing outcome messages. Modules report success or failure here and
//! every UI reads them with its own [`NoticeCursor`].

use std::collections::VecDeque;

use crate::state::{lock, Inner};

const MAX_NOTICES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: NoticeLevel,
    pub text: String,
}

#[derive(Default)]
pub(crate) struct NoticeLog {
    pub(crate) next_seq: u64,
    entries: VecDeque<(u64, Notice)>,
}

impl NoticeLog {
    pub(crate) fn since(&self, cursor: &mut NoticeCursor) -> Vec<Notice> {
        let out = self
            .entries
            .iter()
            .filter(|(seq, _)| *seq >= cursor.0)
            .map(|(_, n)| n.clone())
            .collect();
        cursor.0 = self.next_seq;
        out
    }
}

/// Per-consumer position in the notice log (see [`crate::Service::notices_since`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoticeCursor(pub(crate) u64);

impl Inner {
    pub(crate) fn push_notice(&self, level: NoticeLevel, text: String) {
        match level {
            NoticeLevel::Error => tracing::warn!("{text}"),
            _ => tracing::info!("{text}"),
        }
        {
            let mut log = lock(&self.notices);
            let seq = log.next_seq;
            log.next_seq += 1;
            log.entries.push_back((seq, Notice { level, text }));
            while log.entries.len() > MAX_NOTICES {
                log.entries.pop_front();
            }
        }
        self.notify();
    }
}
