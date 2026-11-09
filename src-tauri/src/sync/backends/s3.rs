//! S3-compatible object store. Only ListBucket and object Get/Put/Delete are used.
use super::cloud;
use crate::sync::{
    blob::{normalize_blob_path, BlobMeta, BlobStore},
    SyncError,
};
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use reqwest::{Method, Url};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Deserialize)]
pub struct S3Config {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default = "yes")]
    pub path_style: bool,
}
fn yes() -> bool {
    true
}
#[derive(Deserialize)]
pub struct S3Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    #[serde(default)]
    pub session_token: Option<String>,
}
pub struct S3Backend {
    config: S3Config,
    credentials: S3Credentials,
    client: reqwest::Client,
}
fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
fn hmac(key: &[u8], value: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(value.as_bytes());
    mac.finalize().into_bytes().to_vec()
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
impl S3Backend {
    pub fn new(mut config: S3Config, credentials: S3Credentials) -> Result<Self, SyncError> {
        let url = Url::parse(&config.endpoint).map_err(cloud::transport)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
        {
            return Err(SyncError::Transport(
                "S3 endpoint must be an HTTPS origin without credentials".into(),
            ));
        }
        if config.region.is_empty()
            || config.bucket.is_empty()
            || !config
                .bucket
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
            || credentials.access_key_id.is_empty()
            || credentials.secret_access_key.is_empty()
        {
            return Err(SyncError::Transport(
                "S3 region, bucket and credentials are required".into(),
            ));
        }
        config.prefix = config.prefix.trim_matches('/').to_owned();
        if !config.prefix.is_empty() {
            normalize_blob_path(&config.prefix)?;
            config.prefix.push('/');
        }
        Ok(Self {
            config,
            credentials,
            client: cloud::client()?,
        })
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        query: Vec<(&str, String)>,
        bytes: &[u8],
    ) -> Result<reqwest::Response, SyncError> {
        let mut url = Url::parse(&self.config.endpoint).map_err(cloud::transport)?;
        let root = if self.config.path_style {
            format!("/{}/", self.config.bucket)
        } else {
            let host = format!(
                "{}.{}",
                self.config.bucket,
                url.host_str().unwrap_or_default()
            );
            url.set_host(Some(&host)).map_err(cloud::transport)?;
            "/".into()
        };
        let encoded_path = path.split('/').map(encode).collect::<Vec<_>>().join("/");
        url.set_path(&format!("{root}{encoded_path}"));
        let mut query: Vec<_> = query
            .into_iter()
            .map(|(k, v)| (encode(k), encode(&v)))
            .collect();
        query.sort();
        let canonical_query = query
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        if !canonical_query.is_empty() {
            url.set_query(Some(&canonical_query));
        }
        let now = chrono::Utc::now();
        let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
        let day = now.format("%Y%m%d").to_string();
        let host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
            None => url.host_str().unwrap_or_default().to_string(),
        };
        let hash = hex(Sha256::digest(bytes));
        let mut headers = std::collections::BTreeMap::from([
            ("host", host),
            ("x-amz-content-sha256", hash.clone()),
            ("x-amz-date", timestamp.clone()),
        ]);
        if let Some(token) = &self.credentials.session_token {
            headers.insert("x-amz-security-token", token.clone());
        }
        let signed = headers.keys().copied().collect::<Vec<_>>().join(";");
        let canonical_headers = headers
            .iter()
            .map(|(k, v)| {
                format!(
                    "{k}:{}\n",
                    v.split_whitespace().collect::<Vec<_>>().join(" ")
                )
            })
            .collect::<String>();
        let canonical_uri = format!(
            "{root}{}",
            path.split('/').map(encode).collect::<Vec<_>>().join("/")
        );
        let canonical = format!(
            "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed}\n{hash}"
        );
        let scope = format!("{day}/{}/s3/aws4_request", self.config.region);
        let string = format!(
            "AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{}",
            hex(Sha256::digest(canonical.as_bytes()))
        );
        let key = hmac(
            format!("AWS4{}", self.credentials.secret_access_key).as_bytes(),
            &day,
        );
        let key = hmac(&key, &self.config.region);
        let key = hmac(&key, "s3");
        let key = hmac(&key, "aws4_request");
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={}",
            self.credentials.access_key_id,
            hex(hmac(&key, &string))
        );
        let mut request = self
            .client
            .request(method, url)
            .header("authorization", authorization)
            .body(bytes.to_vec());
        for (k, v) in headers {
            request = request.header(k, v);
        }
        cloud::send(&self.client, request.build().map_err(cloud::transport)?).await
    }
    fn key(&self, path: &str) -> Result<String, SyncError> {
        Ok(format!(
            "{}{}",
            self.config.prefix,
            normalize_blob_path(path)?
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aws_documented_iam_sigv4_vector() {
        // AWS IAM ListUsers example, 2015-08-30 (public example credentials).
        let canonical = "GET\n/\nAction=ListUsers&Version=2010-05-08\ncontent-type:application/x-www-form-urlencoded; charset=utf-8\nhost:iam.amazonaws.com\nx-amz-date:20150830T123600Z\n\ncontent-type;host;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(
            hex(Sha256::digest(canonical)),
            "f536975d06c0309214f805bb90ccff089219ecd68b2577efef23edd43b7e1a59"
        );
        let key = hmac(b"AWS4wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20150830");
        let key = hmac(&key, "us-east-1");
        let key = hmac(&key, "iam");
        let key = hmac(&key, "aws4_request");
        let string = format!(
            "AWS4-HMAC-SHA256\n20150830T123600Z\n20150830/us-east-1/iam/aws4_request\n{}",
            hex(Sha256::digest(canonical))
        );
        assert_eq!(
            hex(hmac(&key, &string)),
            "5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
    }
    #[tokio::test]
    async fn signed_paginated_list_stays_inside_prefix_and_carries_session_token() {
        let (endpoint, task) = cloud::test_http::server(vec![
            (200, "<ListBucketResult><IsTruncated>true</IsTruncated><Contents><Key>app/v2/a</Key><Size>4</Size></Contents><NextContinuationToken>a+b=</NextContinuationToken></ListBucketResult>".into()),
            (200, "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>app/v2/b</Key><Size>5</Size></Contents><Contents><Key>other/private</Key><Size>9</Size></Contents></ListBucketResult>".into())
        ]).await;
        let backend = S3Backend {
            config: S3Config {
                endpoint,
                region: "test-1".into(),
                bucket: "bucket".into(),
                prefix: "app/".into(),
                path_style: true,
            },
            credentials: S3Credentials {
                access_key_id: "mock-id".into(),
                secret_access_key: "mock-secret".into(),
                session_token: Some("mock-session".into()),
            },
            client: cloud::client().unwrap(),
        };
        let items = backend.list_blobs("v2/").await.unwrap();
        assert_eq!(
            items.iter().map(|m| m.path.as_str()).collect::<Vec<_>>(),
            vec!["v2/a", "v2/b"]
        );
        let requests = task.await.unwrap();
        assert!(requests[1].contains("continuation-token=a%2Bb%3D"));
        assert!(requests[0].contains("AWS4-HMAC-SHA256 Credential=mock-id/"));
        assert!(requests[0].contains("x-amz-security-token: mock-session"));
        assert!(requests[0].contains("prefix=app%2Fv2%2F"));
    }
    #[test]
    fn rejects_insecure_endpoint_and_prefix_escape() {
        let config = S3Config {
            endpoint: "http://s3.invalid".into(),
            region: "test".into(),
            bucket: "bucket".into(),
            prefix: "../private".into(),
            path_style: true,
        };
        assert!(S3Backend::new(
            config,
            S3Credentials {
                access_key_id: "mock".into(),
                secret_access_key: "mock".into(),
                session_token: None
            }
        )
        .is_err());
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Listing {
    #[serde(default)]
    contents: Vec<Object>,
    #[serde(default)]
    is_truncated: bool,
    next_continuation_token: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Object {
    key: String,
    size: u64,
}
#[async_trait]
impl BlobStore for S3Backend {
    async fn list_blobs(&self, prefix: &str) -> Result<Vec<BlobMeta>, SyncError> {
        if !prefix.is_empty() {
            normalize_blob_path(prefix.trim_end_matches('/'))?;
        }
        let mut token = None;
        let mut seen = std::collections::HashSet::new();
        let mut result = std::collections::BTreeMap::new();
        loop {
            let mut query = vec![
                ("list-type", "2".into()),
                ("prefix", format!("{}{prefix}", self.config.prefix)),
            ];
            if let Some(t) = token {
                query.push(("continuation-token", t));
            }
            let response = self.request(Method::GET, "", query, &[]).await?;
            let body = response.text().await.map_err(cloud::transport)?;
            let page: Listing = quick_xml::de::from_str(&body).map_err(cloud::transport)?;
            for object in page.contents {
                if let Some(path) = object.key.strip_prefix(&self.config.prefix) {
                    if path.starts_with(prefix) && normalize_blob_path(path).is_ok() {
                        result.insert(path.to_owned(), object.size);
                    }
                }
            }
            if !page.is_truncated {
                break;
            }
            let next = page
                .next_continuation_token
                .filter(|t| !t.is_empty() && seen.insert(t.clone()))
                .ok_or_else(|| cloud::transport("pagination cycle"))?;
            token = Some(next);
        }
        Ok(result
            .into_iter()
            .map(|(path, size)| BlobMeta { path, size })
            .collect())
    }
    async fn get_blob(&self, path: &str) -> Result<Vec<u8>, SyncError> {
        let r = self
            .request(Method::GET, &self.key(path)?, vec![], &[])
            .await?;
        if !r.status().is_success() {
            return Err(cloud::transport("missing"));
        }
        Ok(r.bytes().await.map_err(cloud::transport)?.to_vec())
    }
    async fn put_blob(&self, path: &str, bytes: &[u8]) -> Result<(), SyncError> {
        self.request(Method::PUT, &self.key(path)?, vec![], bytes)
            .await?;
        Ok(())
    }
    async fn delete_blob(&self, path: &str) -> Result<(), SyncError> {
        self.request(Method::DELETE, &self.key(path)?, vec![], &[])
            .await?;
        Ok(())
    }
}
