//! Check latest GitHub release version

#![deny(
    clippy::all,
    clippy::pedantic,
    missing_debug_implementations,
    missing_docs,
    trivial_casts,
    trivial_numeric_casts,
    unsafe_code,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results
)]

use log::debug;
use reqwest::{
    blocking::{Client, ClientBuilder},
    header::{self, HeaderMap},
};
use serde::Deserialize;
use thiserror::Error;

/// Errors that may be raised by this crate.
#[derive(Debug, Error)]
pub enum LookupError {
    /// May arise from working with the HTTP client.
    #[error("HTTP client error")]
    HttpClient(#[from] reqwest::Error),
    /// May arise from working with the HTTP client.
    #[error("invalid header value")]
    HeaderValue(#[from] reqwest::header::InvalidHeaderValue),
    /// May arise from working with the HTTP client.
    #[error("could not get header value")]
    HeaderToString(#[from] reqwest::header::ToStrError),
    /// May arise if the repository does not have any releases.
    #[error("no release found")]
    NoReleases,
    /// May arise from a mis-supplied repository, or from not having access.
    #[error("repository not found")]
    RepositoryNotFound,
    /// May arise from GitHub API missing or incorrect authentication.
    #[error("authentication error")]
    AuthenticationError(u16),
    /// May arise if GitHub returns an error code from the lookup.
    #[error("received error HTTP response code")]
    ErrorHttpResponse(u16),
}

type Result<T> = std::result::Result<T, LookupError>;

const DEFAULT_USER_AGENT: &'static str = "github.com/celeo/github_version_check";
const DEFAULT_ACCEPT_HEADER: &'static str = "application/vnd.github.v3+json";
const PAGINATION_REQUEST_AMOUNT: usize = 100;

/// The default GitHub instance API root endpoint.
pub const DEFAULT_API_ROOT: &'static str = "https://api.github.com/";

/// Generate the headers required to send HTTP requests to GitHub.
fn generate_headers(token: Option<&str>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let _ = headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_str(DEFAULT_USER_AGENT)?,
    );
    let _ = headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_str(DEFAULT_ACCEPT_HEADER)?,
    );
    if let Some(t) = token {
        let _ = headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {}", t))?,
        );
    }
    Ok(headers)
}

/// Data in the GitHub API response.
#[derive(Debug, Deserialize)]
struct GitHubReleaseItem {
    tag_name: String,
}

/// Struct to communicate with the GitHub REST API.
#[derive(Debug)]
pub struct GitHub {
    client: Client,
    api_root: String,
}

impl GitHub {
    /// Create a new instance of the struct suitable for public GitHub.
    ///
    /// The struct created by this function does not set an access token
    /// and as such can only get information on public GitHub repositories.
    ///
    /// If you need to access information for private repositories or any
    /// information from a custom GitHub enterprise instance, use the
    /// `from_custom` function.
    ///
    /// This function may return an `Error` if the HTTP client could
    /// not be constructed or headers initialized. It should be safe
    /// to unwrap the `Result`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use github_release_check::GitHub;
    /// let github = GitHub::new().unwrap();
    /// ```
    pub fn new() -> Result<Self> {
        let client = ClientBuilder::new()
            .default_headers(generate_headers(None)?)
            .build()?;
        Ok(Self {
            client,
            api_root: DEFAULT_API_ROOT.to_owned(),
        })
    }

    /// Create a new instance of the struct suitable for accessing any GitHub repository
    /// that can be viewed with the access key on the GitHub instance.
    ///
    /// This function has to be used to construct the struct instance whenever the repository
    /// that you want to get information from is on a custom GitHub enterprise instance and/or
    /// is private. The access token passed to this function should be a [GitHub personal access token]
    /// that has the access to view the repository on that GitHub instance.
    ///
    /// For the `api_endpoint` argument, pass in the REST API root of the GitHub instance. For public
    /// GitHub, this can be found in `github_release_check::DEFAULT_API_ROOT`: https://api.github.com/.
    /// Your GitHub enterprise may use a subdomain, or perhaps something like https://github.your_domain_root.com/api/v3/.
    /// Specify the API root that you can otherwise send requests to. Note that this URL should end in a trailing slash.
    ///
    /// # Example
    ///
    /// ```rust
    /// use github_release_check::GitHub;
    /// let github = GitHub::from_custom("https://github.example.com/api/v3/", "abcdef").unwrap();
    /// ```
    ///
    /// [GitHub personal access token]: https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/creating-a-personal-access-token
    pub fn from_custom(api_endpoint: &str, access_token: &str) -> Result<Self> {
        let client = ClientBuilder::new()
            .default_headers(generate_headers(Some(access_token))?)
            .build()?;
        Ok(Self {
            client,
            api_root: api_endpoint.to_owned(),
        })
    }

    /// Get all release versions from the repository.
    ///
    /// Just like `get_latest_version` but instead returns
    /// all the versions in case you need them.
    ///
    /// Actually called by `get_latest_version` under the hood.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use github_release_check::GitHub;
    /// let github = GitHub::new().unwrap();
    /// let versions_result = github.get_all_versions("celeo/github_release_check");
    /// ```
    pub fn get_all_versions(&self, repository: &str) -> Result<Vec<String>> {
        let mut page = 1usize;
        let mut pages = Vec::<Vec<GitHubReleaseItem>>::new();
        let mut last_page: Option<usize> = None;

        loop {
            let query = vec![("per_page", PAGINATION_REQUEST_AMOUNT), ("page", page)];
            let url = format!("{}repos/{}/releases", self.api_root, repository);
            debug!(
                "Querying GitHub at {}, page {} of {}",
                url,
                page,
                last_page
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| String::from("?"))
            );
            let request = self
                .client
                .request(reqwest::Method::GET, &url)
                .query(&query)
                .build()?;
            let response = self.client.execute(request)?;
            if !response.status().is_success() {
                debug!("Got status {} from GitHub release check", response.status());
                let stat = response.status().as_u16();
                if stat == 404 {
                    return Err(LookupError::RepositoryNotFound);
                }
                if stat == 401 || stat == 403 {
                    return Err(LookupError::AuthenticationError(stat));
                }
                return Err(LookupError::ErrorHttpResponse(stat));
            }
            if last_page.is_none() {
                last_page = get_last_page(response.headers())?;
            }
            pages.push(response.json()?);
            page += 1;
            match last_page {
                Some(last) => {
                    debug!("Completed page {} of {}", page, last);
                    if page >= last {
                        break;
                    }
                }
                None => {
                    debug!("No pagination header found (less than 100 releases)");
                    break;
                }
            }
        }

        Ok(pages
            .iter()
            .flat_map(|page| page.iter().map(|item| item.tag_name.clone()))
            .collect())
    }

    /// Get the latest release version from the repository.
    ///
    /// Note that `repository` should be in the format "owner/repo",
    /// like "celeo/github_release_check".
    ///
    /// Since this call can fail for a number of reasons including anything related to
    /// the network at the time of the call, the `Result` from this function should
    /// be handled appropriately.
    ///
    /// This function can fail due to any sort of network issue, invalid access,
    /// or if no releases were found for the repository.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use github_release_check::GitHub;
    /// let github = GitHub::new().unwrap();
    /// let version_result = github.get_latest_version("celeo/github_release_check");
    /// ```
    pub fn get_latest_version(&self, repository: &str) -> Result<String> {
        let versions = self.get_all_versions(repository)?;
        let latest = versions
            .iter()
            .max()
            .ok_or_else(|| LookupError::NoReleases)?;
        Ok(latest.to_owned())
    }
}

// Link: <https://api.github.com/repositories/275449421/releases?per_page=1&page=2>; rel="next", <https://api.github.com/repositories/275449421/releases?per_page=1&page=10>; rel="last"

/// Determine the last page (if any) from the GitHub response headers.
fn get_last_page(headers: &HeaderMap) -> Result<Option<usize>> {
    let links = match headers.get("Link") {
        Some(l) => l.to_str()?,
        None => return Ok(None),
    };
    for part in links.split(',') {
        if part.contains("rel=\"last\"") {
            // TODO
        }
    }
    Ok(None)
}
