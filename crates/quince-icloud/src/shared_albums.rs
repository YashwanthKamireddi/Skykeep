//! Shared Albums, as icloud.com keeps them: not CloudKit but the `sharedstreams` web service.
//! Every call is `POST {sharedstreams}/{dsid}/sharedstreams/{endpoint}` with a JSON body sent as
//! plain text (icloud.com's own way, `main.js:20440`), the session's cookies and query params.
//! Mapped and proven live on throwaways (probe/examples/shared_album_roundtrip.rs; see
//! docs/icloud-web/photos.md).

use crate::{Error, Result};
use serde_json::{json, Value};

/// A shared album, as its list gives it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedAlbum {
    pub guid: String,
    pub name: String,
    /// When it was made, unix seconds.
    pub created_at: Option<i64>,
    /// Made by this account (`sharingtype` "owned"), as against one it was invited to.
    pub owned: bool,
    /// Anyone with its link can see it (`ispublic`).
    pub public: bool,
    /// Those invited can add photos too (`allowcontributions`).
    pub contributions: bool,
}

/// The service, for one signed-in account.
pub struct SharedStreams<'a> {
    pub http: &'a reqwest::Client,
    /// The `sharedstreams` service's address, as the session gives it.
    pub root: &'a str,
    pub params: &'a [(String, String)],
}

impl SharedStreams<'_> {
    fn dsid(&self) -> Result<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == "dsid")
            .map(|(_, v)| v.as_str())
            .ok_or_else(|| Error::Protocol("the session has no dsid".into()))
    }

    async fn call(&self, endpoint: &str, body: Value) -> Result<Value> {
        let url = format!(
            "{}/{}/sharedstreams/{endpoint}",
            self.root.trim_end_matches('/'),
            self.dsid()?
        );
        let r = self
            .http
            .post(&url)
            .query(self.params)
            .header("content-type", "text/plain;charset=UTF-8")
            .body(body.to_string())
            .send()
            .await?;
        let status = r.status();
        let text = r.text().await?;
        if status.as_u16() == 401 || crate::session_lapsed(status) {
            return Err(Error::ReauthRequired);
        }
        if !status.is_success() {
            return Err(Error::Refused(format!(
                "Shared Albums refused that ({status})."
            )));
        }
        // Some answers are a bare word ("OK") rather than JSON.
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }

    /// Every shared album this account has, made or joined, newest first.
    pub async fn list(&self) -> Result<Vec<SharedAlbum>> {
        let v = self.call("webgetalbumslist", json!({})).await?;
        let mut out: Vec<SharedAlbum> = v["albums"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(album_of)
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.name.cmp(&b.name)));
        Ok(out)
    }

    /// A new shared album, as icloud.com makes one: its own id chosen here (upper-case UUID), the
    /// people it is for able to add photos. Answers its id.
    pub async fn create(&self, name: &str) -> Result<String> {
        let guid = uuid::Uuid::new_v4().to_string().to_uppercase();
        let v = self
            .call("createalbum", json!({ "albumguid": guid, "attributes": { "name": name, "allowcontributions": "1" } }))
            .await?;
        if v["albumguid"].as_str() != Some(guid.as_str()) {
            return Err(Error::Protocol(
                "Shared Albums did not say the album was made".into(),
            ));
        }
        Ok(guid)
    }

    /// A shared album deleted, for everyone it was shared with. `"OK"` is Apple's answer.
    pub async fn delete(&self, guid: &str) -> Result<()> {
        let v = self
            .call("deletealbum", json!({ "albumguid": guid }))
            .await?;
        if v.as_str()
            .is_some_and(|s| s.trim().eq_ignore_ascii_case("ok"))
            || v.is_object()
        {
            Ok(())
        } else {
            Err(Error::Protocol(format!("Shared Albums answered {v}")))
        }
    }
}

fn album_of(a: &Value) -> Option<SharedAlbum> {
    let attr = &a["attributes"];
    let on = |k: &str| attr[k].as_str() == Some("1");
    Some(SharedAlbum {
        guid: a["albumguid"].as_str()?.to_string(),
        name: attr["name"].as_str().unwrap_or("Shared Album").to_string(),
        created_at: attr["creationDate"].as_i64().map(|ms| ms / 1000),
        owned: a["sharingtype"].as_str() == Some("owned"),
        public: on("ispublic"),
        contributions: on("allowcontributions"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn params() -> Vec<(String, String)> {
        vec![
            ("dsid".into(), "42".into()),
            ("clientId".into(), "c".into()),
        ]
    }

    #[tokio::test]
    async fn lists_makes_and_deletes_as_icloud_com_does() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/42/sharedstreams/webgetalbumslist"))
            .and(header("content-type", "text/plain;charset=UTF-8"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "albums": [
                { "albumguid": "A", "sharingtype": "owned", "attributes": { "name": "Trip", "creationDate": 2000000, "allowcontributions": "1", "ispublic": "0" } },
                { "albumguid": "B", "sharingtype": "subscribed", "attributes": { "name": "Family", "creationDate": 3000000, "ispublic": "1" } }
            ] })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/42/sharedstreams/createalbum"))
            .and(body_string_contains("\"name\":\"Beach\""))
            .respond_with(|req: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "albumguid": body["albumguid"], "albumctag": "x" }))
            })
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/42/sharedstreams/deletealbum"))
            .and(body_string_contains("\"albumguid\":\"A\""))
            .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let p = params();
        let s = SharedStreams {
            http: &http,
            root: &server.uri(),
            params: &p,
        };
        let list = s.list().await.unwrap();
        assert_eq!(
            list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            vec!["Family", "Trip"],
            "newest first"
        );
        assert!(list[1].owned && list[1].contributions && !list[1].public);
        assert!(!list[0].owned && list[0].public);
        assert_eq!(list[1].created_at, Some(2000));
        let guid = s.create("Beach").await.unwrap();
        assert!(guid.len() == 36 && guid == guid.to_uppercase());
        s.delete("A").await.unwrap();
    }
}
