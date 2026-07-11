//! Bounded, cursor-based pagination for list endpoints (Plan 07, Phase E).
//!
//! Every list endpoint takes a [`PageQuery`] — `limit` (default 50, hard max
//! 200; anything larger is a 400, not a silent clamp) and an opaque `cursor` —
//! and returns a [`Paginated`] envelope whose `next_cursor` resumes the walk.
//! Cursors are **keyset** positions over `(created_at, id)` (finding API-13:
//! offset pagination drifts under concurrent inserts and degrades on deep
//! pages), encoded as unpadded base64. They are opaque to clients: the format
//! is not contract and may change.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::error::{ApiError, ApiResult};

/// Default page size when the client does not specify one.
pub const DEFAULT_LIMIT: i64 = 50;
/// Hard ceiling; a request above this is rejected with 400.
pub const MAX_LIMIT: i64 = 200;

/// Query parameters accepted by paginated list endpoints.
#[derive(Debug, Default, Deserialize)]
pub struct PageQuery {
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// A validated page request: bounded limit + decoded keyset position.
#[derive(Debug, Clone)]
pub struct Page {
    pub limit: i64,
    /// Exclusive resume position: the `(created_at, id)` of the last row the
    /// client saw (rows strictly after it in the listing order are returned).
    pub after: Option<(String, String)>,
}

impl PageQuery {
    /// Validate the raw query into a [`Page`]. Rejects out-of-range limits and
    /// undecodable cursors with 400.
    pub fn validate(self) -> ApiResult<Page> {
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(ApiError::BadRequest(format!(
                "limit must be between 1 and {MAX_LIMIT} (got {limit})"
            )));
        }
        let after = self.cursor.as_deref().map(decode_cursor).transpose()?;
        Ok(Page { limit, after })
    }
}

/// The uniform list envelope: one page of items plus the cursor that resumes
/// the walk (`null` on the last page).
#[derive(Debug, Serialize, ToSchema)]
pub struct Paginated<T> {
    pub items: Vec<T>,
    /// Pass back as `?cursor=` to fetch the next page; absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl<T> Paginated<T> {
    /// Package one fetched page. `rows` must have been fetched with
    /// `LIMIT page.limit + 1` (the sentinel row detects whether another page
    /// exists without a COUNT); `key_of` extracts the `(created_at, id)`
    /// keyset position of a row.
    pub fn from_rows(
        mut rows: Vec<T>,
        page: &Page,
        key_of: impl Fn(&T) -> (String, String),
    ) -> Self {
        let has_more = rows.len() as i64 > page.limit;
        if has_more {
            rows.truncate(page.limit as usize);
        }
        let next_cursor = if has_more {
            rows.last().map(|r| {
                let (created_at, id) = key_of(r);
                encode_cursor(&created_at, &id)
            })
        } else {
            None
        };
        Self {
            items: rows,
            next_cursor,
        }
    }
}

/// Encode a keyset position as an opaque cursor.
pub fn encode_cursor(created_at: &str, id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("{created_at}\x1f{id}"))
}

/// Decode an opaque cursor back into its keyset position.
pub fn decode_cursor(cursor: &str) -> ApiResult<(String, String)> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| ApiError::BadRequest("invalid cursor".to_string()))?;
    let text =
        String::from_utf8(bytes).map_err(|_| ApiError::BadRequest("invalid cursor".to_string()))?;
    let (created_at, id) = text
        .split_once('\x1f')
        .ok_or_else(|| ApiError::BadRequest("invalid cursor".to_string()))?;
    Ok((created_at.to_string(), id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trip() {
        let c = encode_cursor("2026-07-11T00:00:00+00:00", "abc-123");
        let (t, i) = decode_cursor(&c).unwrap();
        assert_eq!(t, "2026-07-11T00:00:00+00:00");
        assert_eq!(i, "abc-123");
        assert!(decode_cursor("!!!not-base64!!!").is_err());
        assert!(decode_cursor(&URL_SAFE_NO_PAD.encode("no-separator")).is_err());
    }

    #[test]
    fn limit_validation() {
        assert_eq!(
            PageQuery::default().validate().unwrap().limit,
            DEFAULT_LIMIT
        );
        assert_eq!(
            PageQuery {
                limit: Some(200),
                cursor: None
            }
            .validate()
            .unwrap()
            .limit,
            200
        );
        for bad in [0, -5, 201, 10_000] {
            assert!(PageQuery {
                limit: Some(bad),
                cursor: None
            }
            .validate()
            .is_err());
        }
    }

    #[test]
    fn envelope_sentinel_row_sets_cursor() {
        let page = Page {
            limit: 2,
            after: None,
        };
        // Three rows fetched for limit 2 → a next page exists.
        let rows = vec![("t1", "a"), ("t2", "b"), ("t3", "c")];
        let p = Paginated::from_rows(rows, &page, |r| (r.0.to_string(), r.1.to_string()));
        assert_eq!(p.items.len(), 2);
        let (t, i) = decode_cursor(p.next_cursor.as_deref().unwrap()).unwrap();
        assert_eq!((t.as_str(), i.as_str()), ("t2", "b"));

        // Exactly-limit rows → last page, no cursor.
        let p = Paginated::from_rows(vec![("t1", "a"), ("t2", "b")], &page, |r| {
            (r.0.to_string(), r.1.to_string())
        });
        assert_eq!(p.items.len(), 2);
        assert!(p.next_cursor.is_none());
    }
}
