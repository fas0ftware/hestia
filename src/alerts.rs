//! ntfy push for checks that got worse or recovered since the last run.

use crate::checks::{Check, Status};

pub fn valid_url(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    matches!(rest, Some(r) if !r.is_empty() && !r.starts_with('/')) && url.len() <= 300 && !url.contains(char::is_whitespace)
}

pub type Post = Box<dyn Fn(&str, &[(String, String)], &str) -> Result<(), String>>;

pub fn ureq_post() -> Post {
    Box::new(|url, headers, body| {
        let mut req = ureq::post(url).timeout(std::time::Duration::from_secs(10));
        for (k, v) in headers {
            req = req.set(k, v);
        }
        req.send_string(body).map(|_| ()).map_err(|e| e.to_string())
    })
}

/// One message per change; returns how many were delivered.
pub fn send(url: &str, host: &str, changes: &[(Check, Status)], post: &Post) -> usize {
    let mut sent = 0;
    for (c, before) in changes {
        let (title, priority, tags) = match c.status {
            Status::Ok => (format!("Hestia: {} recovered on {host}", c.title), "low", "house,white_check_mark"),
            Status::Bad => (format!("Hestia: {} on {host}", c.title), "high", "house,rotating_light"),
            _ => (format!("Hestia: {} on {host}", c.title), "default", "house,warning"),
        };
        let mut body = c.summary.clone();
        if c.status != Status::Ok {
            for d in c.details.iter().take(5) {
                body += &format!("\n· {d}");
            }
        } else {
            body = format!("{} (was {})", c.summary, before.as_str());
        }
        // Header values must be latin-1; everything here is already ASCII, but be strict.
        let title: String = title.chars().filter(|ch| (' '..='~').contains(ch)).collect();
        let headers = vec![
            ("Title".to_string(), title),
            ("Priority".to_string(), priority.to_string()),
            ("Tags".to_string(), tags.to_string()),
        ];
        match post(url, &headers, &body) {
            Ok(()) => sent += 1,
            Err(e) => eprintln!("(ntfy failed: {e})"),
        }
    }
    sent
}
