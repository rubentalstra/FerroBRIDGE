// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AQL query endpoint and its paging.
//!
//! The `POST` form is the one the bridge uses:
//! `docs/specs/its-rest/computable/OAS/query-codegen.openapi.yaml` recommends
//! it over `GET`, because "Requests based on the `GET` method have URI length
//! restriction".

use futures_core::Stream;
use openehr_its::rest::generated::query::AdhocQueryExecute;
use openehr_its::rest::generated::query::QueryExecuteAdhocQueryBodyParams;
use openehr_its::rest::generated::query::ResultSetRow;
use openehr_its::rest::generated::query::client::QueryClient;
use openehr_its::rest::generated::query::client::QueryExecuteAdhocQueryBodyOutcome;

use crate::cdr::Answered;
use crate::cdr::CdrClient;
use crate::cdr::error::CdrError;
use crate::cdr::error::Upstream;

/// What went wrong while a page of a paged query was fetched.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum QueryPageError {
    /// The service refused a page with a status the operation documents.
    #[error("the openEHR service refused the AQL query with {0}")]
    Refused(Upstream),
    /// The call did not reach a documented answer.
    #[error(transparent)]
    Call(#[from] CdrError),
}

/// How many rows one page of a paged query asks for.
///
/// `fetch` is "the number of rows to fetch (i.e. limit)"
/// (`query-codegen.openapi.yaml`, `components.parameters.fetch`), and the
/// paging needs it to know when a short page ends the result set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSize(u32);

impl PageSize {
    /// Returns the page size `rows` asks for.
    ///
    /// # Errors
    /// Returns [`PageSizeError`] when `rows` is zero, which would page
    /// forever.
    pub const fn new(rows: u32) -> Result<Self, PageSizeError> {
        if rows == 0 {
            return Err(PageSizeError::Zero);
        }
        Ok(Self(rows))
    }

    /// Returns the page size as the `fetch` member of the request body.
    #[must_use]
    pub fn rows(self) -> i64 {
        i64::from(self.0)
    }
}

/// A page size that cannot page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PageSizeError {
    /// The page size was zero.
    #[error("a page size of zero would never advance")]
    Zero,
}

/// Where the next page starts and what is left of the current one.
struct Paging {
    /// The request template, whose `offset` and `fetch` are replaced per page.
    request: AdhocQueryExecute,
    /// The row number the next page starts at.
    offset: i64,
    /// How many rows each page asks for.
    page: PageSize,
    /// The rows of the current page that have not been yielded yet.
    buffer: std::vec::IntoIter<ResultSetRow>,
    /// Whether the service has no more rows to give.
    exhausted: bool,
}

impl CdrClient {
    /// Executes one ad-hoc AQL query (`query-codegen.openapi.yaml`,
    /// `query_execute_adhoc_query_body`).
    ///
    /// The call is a `POST`, so it is never retried.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn query_aql(
        &self,
        request: &AdhocQueryExecute,
    ) -> Result<Answered<QueryExecuteAdhocQueryBodyOutcome>, CdrError> {
        let client = self.call(crate::cdr::representation())?;
        let params = QueryExecuteAdhocQueryBodyParams {
            accept: None,
            content_type: None,
        };
        let answered = QueryClient::new(&client)
            .query_execute_adhoc_query_body(&params, request)
            .await;
        crate::cdr::answered_by(&client, answered)
    }

    /// Returns the rows of `request`, one page at a time.
    ///
    /// Each page is one `POST /query/aql` with `offset` advanced by `page`,
    /// the paging ITS-REST 1.1.0 defines for `offset` and `fetch`. The stream
    /// ends when a page comes back shorter than `page`, and a refusal of any
    /// page ends it with [`QueryPageError`] instead of an empty tail. The
    /// `offset` of `request` is where the first page starts.
    pub fn query_aql_rows(
        &self,
        request: AdhocQueryExecute,
        page: PageSize,
    ) -> impl Stream<Item = Result<ResultSetRow, QueryPageError>> + '_ {
        let state = Paging {
            offset: request.offset.unwrap_or_default(),
            request,
            page,
            buffer: Vec::new().into_iter(),
            exhausted: false,
        };
        futures_util::stream::try_unfold(state, move |mut state| async move {
            loop {
                if let Some(row) = state.buffer.next() {
                    return Ok(Some((row, state)));
                }
                if state.exhausted {
                    return Ok(None);
                }
                let mut request = state.request.clone();
                request.offset = Some(state.offset);
                request.fetch = Some(state.page.rows());
                let answered = self.query_aql(&request).await?;
                let rows = match answered.outcome {
                    QueryExecuteAdhocQueryBodyOutcome::Ok { body, .. } => body.rows,
                    QueryExecuteAdhocQueryBodyOutcome::BadRequest
                    | QueryExecuteAdhocQueryBodyOutcome::RequestTimeout => {
                        return Err(QueryPageError::Refused(answered.upstream));
                    }
                };
                let fetched = i64::try_from(rows.len()).unwrap_or(i64::MAX);
                if fetched < state.page.rows() {
                    state.exhausted = true;
                }
                if rows.is_empty() {
                    return Ok(None);
                }
                state.offset = state.offset.saturating_add(fetched);
                state.buffer = rows.into_iter();
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "test assertions")]

    use super::{PageSize, PageSizeError};

    #[test]
    fn a_page_size_of_zero_is_refused() {
        assert_eq!(Err(PageSizeError::Zero), PageSize::new(0));
    }

    #[test]
    fn a_page_size_renders_as_the_fetch_member() -> Result<(), PageSizeError> {
        assert_eq!(25, PageSize::new(25)?.rows());
        Ok(())
    }
}
