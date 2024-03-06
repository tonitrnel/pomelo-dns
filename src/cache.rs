#![allow(unused)]
use anyhow::Context;
use hickory_proto::rr::{Record, RecordType};
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_util::bytes::BufMut;

pub struct Inner(LruCache<String, Vec<Record>>);

impl Inner {
    fn with_capacity(capacity: usize) -> Self {
        Self(LruCache::new(NonZeroUsize::new(capacity).unwrap()))
    }

    pub fn put(&mut self, domain: String, rrs: &[Record]) {
        if self.0.contains(&domain) {
            let vec = self.0.get_mut(&domain).unwrap();
            Self::clean_expired(vec);
            vec.extend_from_slice(rrs);
        } else {
            self.0.put(domain, rrs.to_vec());
        }
    }
    pub fn get(&mut self, domain: &str, rtype: RecordType) -> Option<Vec<Record>> {
        let rrs = match self.0.get_mut(domain) {
            Some(rss) => rss,
            None => return None,
        };
        Self::clean_expired(rrs);
        if rrs.is_empty() {
            self.0.pop(domain);
            return None;
        }
        return Some(
            rrs.iter()
                .filter(|rr| rr.record_type() == rtype)
                .cloned()
                .collect::<Vec<_>>(),
        );
    }
    fn clean_expired(rrs: &mut Vec<Record>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        rrs.retain(|rr| rr.ttl() as u64 <= now);
    }
}

pub struct Cache {
    inner: Option<Mutex<Inner>>,
}

impl Cache {
    pub fn enabled(&self) -> bool {
        self.inner.is_some()
    }
    pub fn with_capacity(capacity: usize) -> Self {
        if capacity == 0 {
            Self { inner: None }
        } else {
            Self {
                inner: Some(Mutex::new(Inner::with_capacity(capacity))),
            }
        }
    }
    pub fn access(&self) -> anyhow::Result<Option<MutexGuard<Inner>>> {
        if let Some(inner) = &self.inner {
            Ok(Some(inner.lock().map_err(|err| {
                anyhow::format_err!("Failed to lock cache, reason: {}", err)
            })?))
        } else {
            Ok(None)
        }
    }
}

fn with_index<T, F>(mut f: F) -> impl FnMut(&T) -> bool
where
    F: FnMut(usize, &T) -> bool,
{
    let mut i = 0;
    move |item| (f(i, item), i += 1).0
}
