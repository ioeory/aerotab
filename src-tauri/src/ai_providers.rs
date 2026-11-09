//! Device-local provider registry. Only saved endpoints may receive provider keys.
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn models_use_both_protocol_headers_and_never_return_error_bodies() {
        use axum::{http::HeaderMap, routing::get, Json, Router};
        for anthropic in [false, true] {
            let app = Router::new().route(
                "/models",
                get(move |headers: HeaderMap| async move {
                    if anthropic {
                        assert_eq!(headers["x-api-key"], "fixture-secret");
                        assert_eq!(headers["anthropic-version"], "2023-06-01");
                        assert!(headers.get("authorization").is_none());
                    } else {
                        assert_eq!(headers["authorization"], "Bearer fixture-secret");
                    }
                    Json(json!({"data":[{"id":"z"},{"id":"a"},{"id":"z"}]}))
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let provider = Provider {
                id: "fixture".into(),
                name: "Fixture".into(),
                provider: if anthropic { "anthropic" } else { "openai" }.into(),
                api_base: format!("http://{}", listener.local_addr().unwrap()),
                model: "fixture".into(),
                enabled: true,
            };
            let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            assert_eq!(
                models(&provider, Some("fixture-secret")).await.unwrap(),
                vec!["a", "z"]
            );
            task.abort();
            let app = Router::new().route(
                "/models",
                get(|| async {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        "fixture-secret echoed by server",
                    )
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let provider = Provider {
                api_base: format!("http://{}", listener.local_addr().unwrap()),
                ..provider
            };
            let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            assert_eq!(
                models(&provider, Some("fixture-secret")).await.unwrap_err(),
                "Model HTTP 401"
            );
            task.abort();
        }
    }
    #[tokio::test]
    async fn model_catalog_is_bounded_and_does_not_follow_redirects() {
        use axum::{routing::get, Router};
        for oversized in [false, true] {
            let app = Router::new().route(
                "/models",
                get(move || async move {
                    if oversized {
                        (
                            axum::http::StatusCode::OK,
                            [("location", "/unsafe")],
                            "x".repeat(MAX_TEXT * 8 + 1),
                        )
                    } else {
                        (
                            axum::http::StatusCode::FOUND,
                            [("location", "/unsafe")],
                            String::new(),
                        )
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let provider = Provider {
                id: "fixture".into(),
                name: "Fixture".into(),
                provider: "ollama".into(),
                api_base: format!("http://{}", listener.local_addr().unwrap()),
                model: "fixture".into(),
                enabled: true,
            };
            let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let error = models(&provider, Some("fixture-secret")).await.unwrap_err();
            assert_eq!(
                error,
                if oversized {
                    "Model catalog limit exceeded"
                } else {
                    "Model HTTP 302"
                }
            );
            task.abort();
        }
    }
    #[test]
    fn provider_endpoint_validation_rejects_unsafe_urls_and_secret_fields() {
        let base = json!({"id":"fixture","name":"Fixture","provider":"openai","apiBase":"https://example.com/v1","model":"fixture","enabled":true});
        for url in [
            "http://example.com",
            "https://user:secret@example.com",
            "https://example.com?key=secret",
            "https://example.com/#secret",
        ] {
            let mut value = base.clone();
            value["apiBase"] = json!(url);
            assert!(serde_json::from_value::<Provider>(value)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut value = base;
        value["apiKey"] = json!("secret");
        assert!(serde_json::from_value::<Provider>(value).is_err());
    }
}
use super::*;
const REGISTRY: &str = "ai.providers";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Provider {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub api_base: String,
    pub model: String,
    pub enabled: bool,
}
impl Provider {
    fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || self.name.trim().is_empty()
            || self.name.len() > 128
        {
            return Err("Invalid AI provider identity".into());
        }
        Config {
            provider: self.provider.clone(),
            api_base: self.api_base.clone(),
            model: self.model.clone(),
            ..Config::default()
        }
        .validate()
    }
    fn public(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("provider serialization");
        value["hasKey"] = json!(secret::has_secret(&account(&self.id)));
        value
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Registry {
    providers: Vec<Provider>,
    default_provider_id: Option<String>,
}
fn account(id: &str) -> String {
    format!("ai.providerKey.{id}")
}
fn persist(store: &SettingsStore, registry: &Registry) -> Result<(), String> {
    store
        .local_set(REGISTRY, &serde_json::to_value(registry).map_err(err)?)
        .map_err(err)
}
fn registry(store: &SettingsStore) -> Result<Registry, String> {
    if let Some(value) = store.local_get(REGISTRY).map_err(err)? {
        return serde_json::from_value(value)
            .map_err(|_| "Invalid local AI provider registry".into());
    }
    // Migration is additive: the legacy config/key remain usable by old callers.
    let mut registry = Registry::default();
    if store.local_get(CONFIG).map_err(err)?.is_some() {
        let cfg = config(store)?;
        let provider = Provider {
            id: "legacy".into(),
            name: "Migrated provider".into(),
            provider: cfg.provider,
            api_base: cfg.api_base,
            model: cfg.model,
            enabled: cfg.enabled,
        };
        match secret::get_secret(ACCOUNT) {
            Ok(key) => secret::set_secret(&account(&provider.id), &key)
                .map_err(|_| "Provider key migration needs an available OS keyring")?,
            Err(secret::SecretError::NotFound) => {}
            Err(_) if !secret::has_secret(ACCOUNT) && provider.provider == "ollama" => {}
            Err(_) => return Err("Provider key migration needs an available OS keyring".into()),
        }
        registry.default_provider_id = Some(provider.id.clone());
        registry.providers.push(provider);
    }
    persist(store, &registry)?;
    Ok(registry)
}
fn find(store: &SettingsStore, id: &str) -> Result<Provider, String> {
    registry(store)?
        .providers
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "Unknown AI provider".into())
}
pub(super) fn select(
    store: &SettingsStore,
    id: &str,
    model: Option<&str>,
    mut cfg: Config,
) -> Result<(Config, Option<String>), String> {
    let provider = find(store, id)?;
    if !provider.enabled {
        return Err("AI provider is disabled".into());
    }
    cfg.provider = provider.provider;
    cfg.api_base = provider.api_base;
    cfg.model = model.unwrap_or(&provider.model).to_owned();
    cfg.validate()?;
    let key = match secret::get_secret(&account(id)) {
        Ok(key) => Some(key),
        Err(secret::SecretError::NotFound) => None,
        Err(_) if cfg.provider == "ollama" => None,
        Err(_) => return Err("Provider key is unavailable in OS keyring".into()),
    };
    Ok((cfg, key))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Save {
    provider: Provider,
    key: Option<String>,
    #[serde(default)]
    make_default: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}
async fn models(provider: &Provider, key: Option<&str>) -> Result<Vec<String>, String> {
    provider.validate()?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "Model client unavailable")?;
    let mut request = client.get(format!(
        "{}/models",
        provider.api_base.trim_end_matches('/')
    ));
    if provider.provider == "anthropic" {
        request = request.header("anthropic-version", "2023-06-01");
        if let Some(key) = key {
            request = request.header("x-api-key", key);
        }
    } else if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| "Model connection failed")?;
    if !response.status().is_success() {
        return Err(format!("Model HTTP {}", response.status().as_u16()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Model response unavailable")?
    {
        if bytes.len() + chunk.len() > MAX_TEXT * 8 {
            return Err("Model catalog limit exceeded".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let data: Value = serde_json::from_slice(&bytes).map_err(|_| "Invalid model catalog")?;
    let entries = data["data"].as_array().ok_or("Invalid model catalog")?;
    if entries.len() > 4096 {
        return Err("Model catalog limit exceeded".into());
    }
    let mut result = Vec::new();
    for entry in entries {
        let id = entry["id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or("Invalid model catalog")?;
        result.push(id.to_owned());
    }
    result.sort();
    result.dedup();
    Ok(result)
}
pub(super) async fn handle(
    method: &str,
    params: Value,
    store: &SettingsStore,
) -> Result<Value, String> {
    match method {
        "ai.providers.list" => {
            let registry = registry(store)?;
            Ok(
                json!({"providers":registry.providers.iter().map(Provider::public).collect::<Vec<_>>(),"defaultProviderId":registry.default_provider_id}),
            )
        }
        "ai.providers.save" => {
            let save: Save =
                serde_json::from_value(params).map_err(|_| "Invalid provider request")?;
            save.provider.validate()?;
            let mut registry = registry(store)?;
            let old = registry.providers.iter().find(|p| p.id == save.provider.id);
            if old.is_none()
                && save.key.is_none()
                && secret::has_secret(&account(&save.provider.id))
            {
                return Err(
                    "Provider ID has a retained key; explicitly replace or clear it".into(),
                );
            }
            if old.is_some_and(|p| {
                p.api_base != save.provider.api_base || p.provider != save.provider.provider
            }) && save.key.is_none()
            {
                return Err(
                    "Endpoint changed; explicitly replace or clear the provider key".into(),
                );
            }
            if old.is_none() && registry.providers.len() >= 64 {
                return Err("Provider limit exceeded".into());
            }
            if let Some(key) = save.key {
                if key.len() > 16384 {
                    return Err("Invalid provider key".into());
                }
                if key.is_empty() {
                    secret::clear_secret(Some(&account(&save.provider.id)))
                } else {
                    secret::set_secret(&account(&save.provider.id), &key)
                }
                .map_err(|_| "Provider keyring write failed")?;
            }
            if let Some(index) = registry
                .providers
                .iter()
                .position(|p| p.id == save.provider.id)
            {
                registry.providers[index] = save.provider.clone();
            } else {
                if registry.providers.len() >= 64 {
                    return Err("Provider limit exceeded".into());
                }
                registry.providers.push(save.provider.clone());
            }
            if save.make_default {
                registry.default_provider_id = Some(save.provider.id.clone());
            }
            persist(store, &registry)?;
            Ok(save.provider.public())
        }
        "ai.providers.remove" | "ai.providers.default" => {
            let id: Id = serde_json::from_value(params).map_err(|_| "Invalid provider request")?;
            let mut registry = registry(store)?;
            if !registry.providers.iter().any(|p| p.id == id.id) {
                return Err("Unknown AI provider".into());
            }
            if method == "ai.providers.remove" {
                // Remove persisted authority before best-effort credential cleanup.
                registry.providers.retain(|p| p.id != id.id);
                if registry.default_provider_id.as_ref() == Some(&id.id) {
                    registry.default_provider_id = None;
                }
                persist(store, &registry)?;
                let _ = secret::clear_secret(Some(&account(&id.id)));
            } else {
                registry.default_provider_id = Some(id.id);
                persist(store, &registry)?;
            }
            Ok(Value::Null)
        }
        "ai.providers.models" | "ai.providers.test" => {
            let id: Id = serde_json::from_value(params).map_err(|_| "Invalid provider request")?;
            let provider = find(store, &id.id)?;
            let key = match secret::get_secret(&account(&id.id)) {
                Ok(key) => Some(key),
                Err(secret::SecretError::NotFound) => None,
                Err(_) if provider.provider == "ollama" => None,
                Err(_) => return Err("Provider key is unavailable in OS keyring".into()),
            };
            let result = models(&provider, key.as_deref()).await;
            if method == "ai.providers.test" {
                Ok(match result {
                    Ok(_) => json!({"ok":true,"message":"Provider connection succeeded"}),
                    Err(message) => json!({"ok":false,"message":message}),
                })
            } else {
                Ok(json!({"models":result?}))
            }
        }
        _ => Err("Unknown provider method".into()),
    }
}
