//! Retrieve releases versions of a repository from GitHub.
//!
//! Two functions are exposed by this crate: one to get the latest (Semantic Versioned) version, and
//! another to get all of the release versions (as `String`s).
//!
//! This crate works for public and private GitHub repositories on public GitHub or GitHub enterprise
//! when supplied with a valid [access token] for that repository / environment.
//!
//! The simplest use case is a public repository on github.com:
//!
//! ```rust,no_run
//! use github_release_check::GitHub;
//!
//! let github = GitHub::new().unwrap();
//! let versions = github.get_all_versions("celeo/github_release_check").unwrap();
//! ```
//!
//! If you want to access a private repository on github.com, you'll need an access token for
//! a user who can view that repository:
//!
//! ```rust,no_run
//! use github_release_check::{GitHub, DEFAULT_API_ROOT};
//!
//! let github = GitHub::from_custom(DEFAULT_API_ROOT, "your-access-token").unwrap();
//! let versions = github.get_all_versions("you/private-repo").unwrap();
//! ```
//!
//! If you are using a private GitHub enterprise environment:
//!
//! ```rust,no_run
//! use github_release_check::GitHub;
//!
//! let github = GitHub::from_custom("https://github.your_domain.com/api/v3/", "your-access-token").unwrap();
//! let versions = github.get_all_versions("you/private-repo").unwrap();
//! ```
//!
//! Of course, handling these `Result`s with something other than just unwrapping them is a good idea.
//!
//! If you wish to gain more information on each release, use the `query` function:
//!
//! ```rust,no_run
//! use github_release_check::GitHub;
//!
//! let github = GitHub::new().unwrap();
//! let versions = github.query("celeo/github_release_check").unwrap();
//! ```
//!
//! [access token]: https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/creating-a-personal-access-token

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
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::{
    blocking::{Client, ClientBuilder},
    header::{self, HeaderMap},
};
use semver::Version;
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

const DEFAULT_USER_AGENT: &str = "github.com/celeo/github_version_check";
const DEFAULT_ACCEPT_HEADER: &str = "application/vnd.github.v3+json";
const PAGINATION_REQUEST_AMOUNT: usize = 100;
static PAGE_EXTRACT_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(\w*)page=(\d+)").expect("Could not compile regex"));

/// The default GitHub instance API root endpoint.
///
/// You can use this exported `String` if you want to query
/// a private repository on <https://github.com>.
pub const DEFAULT_API_ROOT: &str = "https://api.github.com/";

/// Generate the headers required to send HTTP requests to GitHub.
fn generate_headers(token: Option<&str>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let _prev = headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_str(DEFAULT_USER_AGENT)?,
    );
    let _prev = headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_str(DEFAULT_ACCEPT_HEADER)?,
    );
    if let Some(t) = token {
        let _prev = headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {t}"))?,
        );
    }
    Ok(headers)
}

/// Data for a release in the GitHub API response.
///
/// For information on the struct keys, see [the GitHub docs].
///
/// [the GitHub docs]: https://docs.github.com/en/rest/releases/releases#list-releases
#[derive(Debug, Deserialize, Clone)]
#[allow(missing_docs)]
pub struct GitHubReleaseItem {
    pub url: String,
    pub assets_url: String,
    pub upload_url: String,
    pub html_url: String,
    pub tag_name: String,
    pub name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub created_at: String,
    pub published_at: String,
    pub body: String,
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
    ///
    /// # Errors
    ///
    /// This function fails if the headers cannot be constructed.
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
    /// GitHub, this can be found in [`DEFAULT_API_ROOT`].
    /// Your GitHub enterprise may use a subdomain like `"https://api.github.your_domain_root.com/"`, or
    /// perhaps something like `"https://github.your_domain_root.com/api/v3/"`. Specify the API root that
    /// you can otherwise send requests to. Note that this URL should end in a trailing slash.
    ///
    /// # Example
    ///
    /// ```rust
    /// use github_release_check::GitHub;
    /// let github = GitHub::from_custom("https://github.example.com/api/v3/", "abcdef").unwrap();
    /// ```
    ///
    /// # Errors
    ///
    /// This function fails if the headers cannot be constructed.
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
    /// Note that `repository` should be in the format "owner/repo",
    /// like `"celeo/github_release_check"`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use github_release_check::GitHub;
    /// let github = GitHub::new().unwrap();
    /// let versions_result = github.get_all_versions("celeo/github_release_check");
    /// ```
    ///
    /// # Errors
    ///
    /// This function fails if the HTTP request cannot be sent, the API returns
    /// a status code indicating something other than a success (outside of the
    /// 2xx range), of if the returned data does not match the expected model.
    pub fn query(&self, repository: &str) -> Result<Vec<GitHubReleaseItem>> {
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
                last_page.map_or_else(|| String::from("?"), |p| p.to_string())
            );
            let request = self
                .client
                .request(reqwest::Method::GET, &url)
                .query(&query)
                .build()?;
            let response = self.client.execute(request)?;
            if !response.status().is_success() {
                debug!(
                    "Got status \"{}\" from GitHub release check",
                    response.status()
                );
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
                debug!("Determining last page from response headers");
                last_page = get_last_page(response.headers())?;
            }
            pages.push(response.json()?);
            page += 1;
            if let Some(last) = last_page {
                if page >= last {
                    break;
                }
            } else {
                debug!("No pagination header found (fewer than 100 releases)");
                break;
            }
        }

        Ok(pages.iter().flatten().cloned().collect())
    }

    /// Get all release version strings from the repository.
    ///
    /// Note that `repository` should be in the format "owner/repo",
    /// like `"celeo/github_release_check"`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use github_release_check::GitHub;
    /// let github = GitHub::new().unwrap();
    /// let versions_result = github.get_all_versions("celeo/github_release_check");
    /// ```
    ///
    /// # Errors
    ///
    /// This function fails if the HTTP request cannot be sent, the API returns
    /// a status code indicating something other than a success (outside of the
    /// 2xx range), of if the returned data does not match the expected model.
    pub fn get_all_versions(&self, repository: &str) -> Result<Vec<String>> {
        Ok(self
            .query(repository)?
            .iter()
            .map(|release| release.tag_name.clone())
            .collect())
    }

    /// Get the latest release version from the repository.
    ///
    /// Note that `repository` should be in the format "owner/repo",
    /// like `"celeo/github_release_check"`.
    ///
    /// As this function needs to select and return the latest release version,
    /// it makes use of the "semver" crate's `Version` [parse function]. As there's
    /// no requirement for repositories to use Semantic Versioning, this function may
    /// not suitable for every repository (thus the `get_all_versions` function which
    /// just works with `String`s).
    ///
    /// A leading `'v'` character is stripped from the versions
    /// in order to make more repositories work. For any version string that is not
    /// able to be loaded into a `Version` struct, it is skipped. Note that this
    /// may result in no or missing versions.
    ///
    /// Effectively, for repositories that are using Semantic Versioning correctly,
    /// this will work. For those that are not, it's a bit of a toss-up.
    ///
    /// Since this call can fail for a number of reasons including anything related to
    /// the network at the time of the call, the `Result` from this function should
    /// be handled appropriately.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use github_release_check::GitHub;
    /// let github = GitHub::new().unwrap();
    /// let version_result = github.get_latest_version("celeo/github_release_check");
    /// ```
    ///
    /// # Errors
    ///
    /// This function fails for any of the reasons in `get_all_versions`, or
    /// if no versions are returned from the API.
    ///
    /// [parse function]: https://docs.rs/semver/latest/semver/struct.Version.html#method.parse
    pub fn get_latest_version(&self, repository: &str) -> Result<Version> {
        let versions = self.get_all_versions(repository)?;
        let latest = versions
            .iter()
            .map(|s| {
                let mut s = s.clone();
                if s.starts_with('v') {
                    s = s.chars().skip(1).collect();
                }
                Version::parse(&s)
            })
            .filter_map(std::result::Result::ok)
            .max()
            .ok_or(LookupError::NoReleases)?;
        Ok(latest)
    }
}

/// Determine the last page (if any) from the GitHub response headers.
///
/// # Errors
///
/// This function fails if the the values in the "link" header
/// are not valid ASCII.
fn get_last_page(headers: &HeaderMap) -> Result<Option<usize>> {
    let links = match headers.get("link") {
        Some(l) => l.to_str()?,
        None => return Ok(None),
    };
    for page_ref in links.split(',') {
        if !page_ref.contains("rel=\"last\"") {
            continue;
        }
        for cap_part in PAGE_EXTRACT_REGEX.captures_iter(page_ref) {
            if cap_part[1].is_empty() {
                let page = cap_part[2]
                    .parse::<usize>()
                    .expect("Could not get page version from regex");
                return Ok(Some(page));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{get_last_page, GitHub};
    use mockito::mock;
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    #[test]
    fn test_get_last_page_none() {
        let map = HeaderMap::new();
        let last = get_last_page(&map).unwrap();
        assert!(last.is_none());
    }

    #[test]
    fn test_get_last_page_some() {
        let mut map = HeaderMap::new();
        let _ = map.insert(
            HeaderName::from_static("link"),
            HeaderValue::from_static(r#"<https://api.github.com/repositories/275449421/releases?per_page=1&page=2>; rel="next", <https://api.github.com/repositories/275449421/releases?per_page=1&page=10>; rel="last""#)
        );
        let last = get_last_page(&map).unwrap();
        assert_eq!(last, Some(10));
    }

    #[test]
    fn test_get_all_versions_none() {
        let _m = mock("GET", "/repos/foo/bar/releases")
            .match_query(mockito::Matcher::Any)
            .with_body("[]")
            .create();
        let github = GitHub::from_custom(&format!("{}/", mockito::server_url()), "").unwrap();
        let versions = github.get_all_versions("foo/bar").unwrap();
        assert!(versions.is_empty());
    }

    #[test]
    fn test_get_all_versions_valid() {
        let rest = r#", "url": "", "assets_url": "", "upload_url": "", "html_url": "", "name": "", "draft": false, "prerelease": false, "created_at": "", "published_at": "", "body": """#;
        let _m = mock("GET", "/repos/foo/bar/releases")
            .match_query(mockito::Matcher::Any)
            .with_body(format!(
                r#"[
                {{ "tag_name": "v1.0.0"  {rest}}},
                {{ "tag_name": "v1.9.10"  {rest}}},
                {{ "tag_name": "v0.3.0" {rest}}}
            ]"#
            ))
            .create();
        let github = GitHub::from_custom(&format!("{}/", mockito::server_url()), "").unwrap();
        let versions = github.get_all_versions("foo/bar").unwrap();
        assert_eq!(versions.len(), 3);
    }

    #[test]
    fn test_get_latest_version_none() {
        let _m = mock("GET", "/repos/foo/bar/releases")
            .match_query(mockito::Matcher::Any)
            .with_body("[]")
            .create();
        let github = GitHub::from_custom(&format!("{}/", mockito::server_url()), "").unwrap();
        let version_res = github.get_latest_version("foo/bar");
        assert!(version_res.is_err());
    }

    #[test]
    fn test_get_latest_version_bad_semvers() {
        let rest = r#", "url": "", "assets_url": "", "upload_url": "", "html_url": "", "name": "", "draft": false, "prerelease": false, "created_at": "", "published_at": "", "body": """#;
        let _m = mock("GET", "/repos/foo/bar/releases")
            .match_query(mockito::Matcher::Any)
            .with_body(format!(
                r#"[
                {{ "tag_name": "uhhhh" {rest}}},
                {{ "tag_name": "v3.0.0-alpha" {rest}}},
                {{ "tag_name": "v1.9.10" {rest}}}
            ]"#
            ))
            .create();
        let github = GitHub::from_custom(&format!("{}/", mockito::server_url()), "").unwrap();
        let version = github.get_latest_version("foo/bar").unwrap();
        assert_eq!(version, semver::Version::parse("3.0.0-alpha").unwrap());
    }
}
