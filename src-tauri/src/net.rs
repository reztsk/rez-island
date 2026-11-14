//! APIs publiques : météo (Open-Meteo), stats par pseudo (fortnite-api.com), chat IA (Claude API).
use crate::err;
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Duration;

pub fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("REZ-Island")
            .timeout(Duration::from_secs(90))
            .build()
            .expect("http client")
    })
}

pub async fn weather(city: &str) -> Result<Value, String> {
    let g: Value = client()
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query(&[("name", city), ("count", "1"), ("language", "fr")])
        .send()
        .await
        .map_err(err)?
        .json()
        .await
        .map_err(err)?;
    let r = g["results"].get(0).ok_or("Ville introuvable")?;
    let lat = r["latitude"].as_f64().unwrap_or(0.0).to_string();
    let lon = r["longitude"].as_f64().unwrap_or(0.0).to_string();
    let w: Value = client()
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&[
            ("latitude", lat.as_str()),
            ("longitude", lon.as_str()),
            ("current", "temperature_2m,apparent_temperature,weather_code,wind_speed_10m,is_day"),
            ("daily", "temperature_2m_max,temperature_2m_min"),
            ("timezone", "auto"),
            ("forecast_days", "1"),
        ])
        .send()
        .await
        .map_err(err)?
        .json()
        .await
        .map_err(err)?;
    let c = &w["current"];
    Ok(json!({
        "city": r["name"],
        "temp": c["temperature_2m"],
        "feels": c["apparent_temperature"],
        "code": c["weather_code"],
        "wind": c["wind_speed_10m"],
        "is_day": c["is_day"],
        "max": w["daily"]["temperature_2m_max"][0],
        "min": w["daily"]["temperature_2m_min"][0],
    }))
}

pub async fn fortnite(key: &str, name: &str, platform: &str, window: &str) -> Result<Value, String> {
    if key.is_empty() {
        return Err("Clé fortnite-api.com manquante (Paramètres)".into());
    }
    let r = client()
        .get("https://fortnite-api.com/v2/stats/br/v2")
        .header("Authorization", key)
        .query(&[("name", name), ("accountType", platform), ("timeWindow", window)])
        .send()
        .await
        .map_err(err)?;
    let ok = r.status().is_success();
    let v: Value = r.json().await.map_err(err)?;
    if !ok {
        return Err(v["error"].as_str().unwrap_or("Joueur introuvable ou profil privé").to_string());
    }
    let d = &v["data"];
    Ok(json!({
        "name": d["account"]["name"],
        "level": d["battlePass"]["level"],
        "overall": d["stats"]["all"]["overall"],
    }))
}

const SYSTEM: &str = "Tu es REZ, l'assistant intégré à une Dynamic Island Windows. \
Réponds en français, de façon courte, claire et directe. Pas de markdown lourd.";

pub async fn claude(key: &str, model: &str, history: Vec<Value>) -> Result<String, String> {
    if key.is_empty() {
        return Err("Clé API Anthropic manquante (Paramètres)".into());
    }
    let model = if model.trim().is_empty() { "claude-opus-5-5" } else { model.trim() };

    // 20 derniers messages, le premier doit être "user"
    let start = history.len().saturating_sub(20);
    let mut msgs: Vec<Value> = history[start..].to_vec();
    while msgs.first().map(|m| m["role"] != "user").unwrap_or(false) {
        msgs.remove(0);
    }
    if msgs.is_empty() {
        return Err("Message vide".into());
    }

    let mut body = json!({
        "model": model,
        "max_tokens": 16000,
        "system": SYSTEM,
        "messages": msgs,
    });
    let gen5 = model.starts_with("claude-opus-5")
        || model.starts_with("claude-sonnet-5")
        || model.starts_with("claude-haiku-5")
        || model.starts_with("claude-fable-5");
    if gen5 {
        // chat d'overlay : réponses rapides
        body["output_config"] = json!({ "effort": "low" });
    }
    let mut req = client()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", key)
        .header("anthropic-version", "2026-10-21");
    let fallback = model.starts_with("claude-opus-5") || model.starts_with("claude-sonnet-5-5") || model.starts_with("claude-fable-5");
    if fallback {
        // repli serveur automatique si le modèle refuse
        body["fallbacks"] = json!("default");
        req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
    }

    let r = req.json(&body).send().await.map_err(err)?;
    let status = r.status();
    let v: Value = r.json().await.map_err(err)?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("erreur inconnue");
        return Err(format!("API {} : {msg}", status.as_u16()));
    }
    if v["stop_reason"] == "refusal" {
        return Err("Requête refusée par le modèle.".into());
    }
    let text = v["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    Ok(text)
}
