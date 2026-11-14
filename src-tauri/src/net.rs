//! APIs publiques : météo (Open-Meteo), pochettes (iTunes), stats par pseudo (fortnite-api.com,
//! HenrikDev Valorant, apexlegendsapi.com), chat IA (Claude, ChatGPT, Gemini), OAuth Discord. Aucun accès aux jeux.
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

async fn get_json(req: reqwest::RequestBuilder) -> Result<(bool, Value), String> {
    let r = req.send().await.map_err(err)?;
    let ok = r.status().is_success();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    Ok((ok, v))
}

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Météo temps réel Open-Meteo (sans clé) pour la ville saisie.
pub async fn weather(city: &str) -> Result<Value, String> {
    let city = city.trim();
    if city.is_empty() {
        return Err("Entre une ville".into());
    }
    let (_, g) = get_json(
        client()
            .get("https://geocoding-api.open-meteo.com/v1/search")
            .query(&[("name", city), ("count", "1"), ("language", "fr"), ("format", "json")]),
    )
    .await?;
    let r = g["results"].get(0).ok_or_else(|| format!("Ville introuvable : {city}"))?;
    let lat = r["latitude"].as_f64().unwrap_or(0.0).to_string();
    let lon = r["longitude"].as_f64().unwrap_or(0.0).to_string();
    let (ok, w) = get_json(client().get("https://api.open-meteo.com/v1/forecast").query(&[
        ("latitude", lat.as_str()),
        ("longitude", lon.as_str()),
        (
            "current",
            "temperature_2m,apparent_temperature,weather_code,wind_speed_10m,is_day,relative_humidity_2m,precipitation",
        ),
        ("hourly", "temperature_2m,weather_code,precipitation_probability,is_day"),
        (
            "daily",
            "temperature_2m_max,temperature_2m_min,weather_code,uv_index_max,sunrise,sunset,precipitation_probability_max",
        ),
        ("timezone", "auto"),
        ("forecast_days", "5"),
    ]))
    .await?;
    if !ok {
        return Err("Météo indisponible".into());
    }
    let c = &w["current"];
    // 8 prochaines heures à partir de l'heure courante (heure locale de la ville)
    let now = c["time"].as_str().unwrap_or("").get(..13).unwrap_or("").to_string();
    let h = &w["hourly"];
    let times = h["time"].as_array().cloned().unwrap_or_default();
    let i0 = times
        .iter()
        .position(|t| t.as_str().map(|s| s.starts_with(&now)).unwrap_or(false))
        .unwrap_or(0);
    let hourly: Vec<Value> = (i0..(i0 + 8).min(times.len()))
        .map(|i| {
            json!({
                "time": times[i].as_str().unwrap_or("").get(11..13).unwrap_or(""),
                "temp": h["temperature_2m"][i],
                "code": h["weather_code"][i],
                "rain": h["precipitation_probability"][i],
                "is_day": h["is_day"][i],
            })
        })
        .collect();
    let d = &w["daily"];
    let days: Vec<Value> = (0..d["time"].as_array().map(|a| a.len()).unwrap_or(0))
        .map(|i| {
            json!({
                "date": d["time"][i],
                "max": d["temperature_2m_max"][i],
                "min": d["temperature_2m_min"][i],
                "code": d["weather_code"][i],
                "rain": d["precipitation_probability_max"][i],
            })
        })
        .collect();
    let hm = |v: &Value| v.as_str().unwrap_or("").get(11..16).unwrap_or("").to_string();
    Ok(json!({
        "city": r["name"],
        "country": r["country_code"],
        "region": r["admin1"],
        "temp": c["temperature_2m"],
        "feels": c["apparent_temperature"],
        "code": c["weather_code"],
        "wind": c["wind_speed_10m"],
        "is_day": c["is_day"],
        "humidity": c["relative_humidity_2m"],
        "precip": c["precipitation"],
        "max": d["temperature_2m_max"][0],
        "min": d["temperature_2m_min"][0],
        "uv": d["uv_index_max"][0],
        "sunrise": hm(&d["sunrise"][0]),
        "sunset": hm(&d["sunset"][0]),
        "hourly": hourly,
        "days": days,
    }))
}

/// Pochette HD (iTunes Search, public, sans clé). Vide si l'artiste ne correspond pas.
pub async fn cover_hd(title: &str, artist: &str) -> Result<String, String> {
    let (t, a) = (title.trim(), artist.trim());
    if t.is_empty() || a.is_empty() {
        return Ok(String::new());
    }
    let term = format!("{a} {t}");
    let (_, v) = get_json(client().get("https://itunes.apple.com/search").query(&[
        ("term", term.as_str()),
        ("media", "music"),
        ("entity", "song"),
        ("limit", "5"),
    ]))
    .await?;
    let al = a.to_lowercase();
    let first = al.split([',', '&']).next().unwrap_or(&al).trim().to_string();
    let hit = v["results"].as_array().and_then(|rs| {
        rs.iter().find(|r| {
            let ra = r["artistName"].as_str().unwrap_or("").to_lowercase();
            !ra.is_empty() && (ra.contains(&first) || first.contains(&ra))
        })
    });
    Ok(hit
        .and_then(|r| r["artworkUrl100"].as_str())
        .map(|u| u.replace("100x100bb", "600x600bb"))
        .unwrap_or_default())
}

pub async fn fortnite(key: &str, name: &str, platform: &str, window: &str) -> Result<Value, String> {
    if key.is_empty() {
        return Err("Clé fortnite-api.com manquante (Paramètres)".into());
    }
    let (ok, v) = get_json(
        client()
            .get("https://fortnite-api.com/v2/stats/br/v2")
            .header("Authorization", key)
            .query(&[("name", name), ("accountType", platform), ("timeWindow", window)]),
    )
    .await?;
    if !ok {
        return Err(v["error"].as_str().unwrap_or("Joueur introuvable ou profil privé").to_string());
    }
    let d = &v["data"];
    let all = &d["stats"]["all"];
    Ok(json!({
        "name": d["account"]["name"],
        "level": d["battlePass"]["level"],
        "overall": all["overall"],
        "solo": all["solo"],
        "duo": all["duo"],
        "squad": all["squad"],
    }))
}

fn henrik(key: &str, path: &str) -> reqwest::RequestBuilder {
    client()
        .get(format!("https://api.henrikdev.xyz/valorant/{path}"))
        .header("Authorization", key)
}

fn henrik_err(v: &Value) -> String {
    v["errors"][0]["message"]
        .as_str()
        .or(v["message"].as_str())
        .unwrap_or("Joueur introuvable")
        .to_string()
}

/// Valorant (HenrikDev API, données publiques Riot) : rang + dernière partie avec alliés / adversaires.
pub async fn valorant(key: &str, riot_id: &str, region: &str) -> Result<Value, String> {
    if key.is_empty() {
        return Err("Clé HenrikDev manquante (Paramètres)".into());
    }
    let (name, tag) = riot_id.split_once('#').ok_or("Format attendu : Pseudo#TAG")?;
    let (name, tag) = (urlenc(name.trim()), urlenc(tag.trim()));
    let (ok, acc) = get_json(henrik(key, &format!("v1/account/{name}/{tag}"))).await?;
    if !ok {
        return Err(henrik_err(&acc));
    }
    let a = &acc["data"];
    let region = a["region"].as_str().filter(|r| !r.is_empty()).unwrap_or(region).to_string();
    let puuid = a["puuid"].as_str().unwrap_or("").to_string();
    let rank = match get_json(henrik(key, &format!("v2/mmr/{region}/{name}/{tag}"))).await {
        Ok((true, v)) => v["data"]["current_data"].clone(),
        _ => Value::Null,
    };
    let mut out = json!({
        "name": a["name"], "tag": a["tag"], "level": a["account_level"], "region": region,
        "card": a["card"]["wide"],
        "rank": rank["currenttierpatched"], "rr": rank["ranking_in_tier"], "rank_img": rank["images"]["small"],
        "last": Value::Null,
    });
    let Ok((true, mv)) = get_json(henrik(key, &format!("v3/matches/{region}/{name}/{tag}?size=1"))).await else {
        return Ok(out);
    };
    let Some(m) = mv["data"].get(0) else { return Ok(out) };
    let players = m["players"]["all_players"].as_array().cloned().unwrap_or_default();
    let me = players.iter().find(|p| p["puuid"] == puuid.as_str()).cloned().unwrap_or(Value::Null);
    let my_team = me["team"].as_str().unwrap_or("").to_string();
    let slim = |p: &Value| {
        json!({
            "name": format!("{}#{}", p["name"].as_str().unwrap_or("?"), p["tag"].as_str().unwrap_or("")),
            "agent": p["character"], "icon": p["assets"]["agent"]["small"], "rank": p["currenttier_patched"],
            "k": p["stats"]["kills"], "d": p["stats"]["deaths"], "a": p["stats"]["assists"], "score": p["stats"]["score"],
            "me": p["puuid"] == puuid.as_str(),
        })
    };
    let (mut allies, mut enemies): (Vec<Value>, Vec<Value>) = (vec![], vec![]);
    for p in &players {
        let mine = p["puuid"] == puuid.as_str();
        let same = !my_team.is_empty() && my_team != "Neutral" && p["team"].as_str() == Some(my_team.as_str());
        if mine || same {
            allies.push(slim(p));
        } else {
            enemies.push(slim(p));
        }
    }
    for v in [&mut allies, &mut enemies] {
        v.sort_by_key(|p| -(p["score"].as_i64().unwrap_or(0)));
    }
    let t = my_team.to_lowercase();
    let other = if t == "red" { "blue" } else { "red" };
    let teams = &m["teams"];
    let score = if teams[&t].is_object() {
        json!(format!(
            "{} - {}",
            teams[&t]["rounds_won"].as_i64().unwrap_or(0),
            teams[other]["rounds_won"].as_i64().unwrap_or(0)
        ))
    } else {
        Value::Null
    };
    out["last"] = json!({
        "map": m["metadata"]["map"], "mode": m["metadata"]["mode"], "date": m["metadata"]["game_start_patched"],
        "won": teams[&t]["has_won"], "score": score,
        "me": if me.is_null() { Value::Null } else { slim(&me) },
        "allies": allies, "enemies": enemies,
    });
    Ok(out)
}

/// Apex Legends (apexlegendsapi.com, public) : rang, niveau, statut en direct (en partie / lobby).
pub async fn apex(key: &str, name: &str, platform: &str) -> Result<Value, String> {
    if key.is_empty() {
        return Err("Clé apexlegendsapi.com manquante (Paramètres)".into());
    }
    let (_, v) = get_json(client().get("https://api.mozambiquehe.re/bridge").query(&[
        ("auth", key),
        ("player", name),
        ("platform", platform),
    ]))
    .await?;
    if let Some(e) = v["Error"].as_str().or(v["error"].as_str()) {
        return Err(e.to_string());
    }
    let g = &v["global"];
    if g.is_null() {
        return Err("Joueur introuvable".into());
    }
    let rt = &v["realtime"];
    let sel = &v["legends"]["selected"];
    let div = g["rank"]["rankDiv"].as_i64().filter(|d| *d > 0).map(|d| format!(" {d}")).unwrap_or_default();
    Ok(json!({
        "name": g["name"], "level": g["level"], "platform": g["platform"],
        "rank": format!("{}{div}", g["rank"]["rankName"].as_str().unwrap_or("—")),
        "rp": g["rank"]["rankScore"], "rank_img": g["rank"]["rankImg"],
        "online": rt["isOnline"].as_i64().unwrap_or(0) == 1,
        "in_game": rt["isInGame"].as_i64().unwrap_or(0) == 1,
        "party_full": rt["partyFull"].as_i64().unwrap_or(0) == 1,
        "state": rt["currentStateAsText"],
        "legend": rt["selectedLegend"].as_str().or(sel["LegendName"].as_str()),
        "legend_img": sel["ImgAssets"]["icon"],
        "legend_stats": sel["data"],
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

/// 20 derniers messages, en commençant par l'utilisateur.
fn trim_history(history: Vec<Value>) -> Result<Vec<Value>, String> {
    let start = history.len().saturating_sub(20);
    let mut msgs: Vec<Value> = history[start..].to_vec();
    while msgs.first().map(|m| m["role"] != "user").unwrap_or(false) {
        msgs.remove(0);
    }
    if msgs.is_empty() {
        return Err("Message vide".into());
    }
    Ok(msgs)
}

fn api_err(status: reqwest::StatusCode, v: &Value) -> String {
    let msg = v["error"]["message"].as_str().or(v["error"].as_str()).unwrap_or("erreur inconnue");
    format!("API {} : {msg}", status.as_u16())
}

/// ChatGPT (API OpenAI, Chat Completions).
pub async fn openai(key: &str, model: &str, history: Vec<Value>) -> Result<String, String> {
    if key.is_empty() {
        return Err("Clé API OpenAI manquante".into());
    }
    let model = if model.trim().is_empty() { "gpt-5-mini" } else { model.trim() };
    let mut msgs = vec![json!({ "role": "system", "content": SYSTEM })];
    msgs.extend(trim_history(history)?);
    let r = client()
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(key)
        .json(&json!({ "model": model, "messages": msgs }))
        .send()
        .await
        .map_err(err)?;
    let status = r.status();
    let v: Value = r.json().await.map_err(err)?;
    if !status.is_success() {
        return Err(api_err(status, &v));
    }
    Ok(v["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
}

/// Gemini (API Google AI Studio).
pub async fn gemini(key: &str, model: &str, history: Vec<Value>) -> Result<String, String> {
    if key.is_empty() {
        return Err("Clé API Gemini manquante".into());
    }
    let model = if model.trim().is_empty() { "gemini-2.5-flash" } else { model.trim() };
    let contents: Vec<Value> = trim_history(history)?
        .iter()
        .map(|m| {
            let role = if m["role"] == "assistant" { "model" } else { "user" };
            json!({ "role": role, "parts": [{ "text": m["content"] }] })
        })
        .collect();
    let r = client()
        .post(format!("https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent", urlenc(model)))
        .header("x-goog-api-key", key)
        .json(&json!({ "system_instruction": { "parts": [{ "text": SYSTEM }] }, "contents": contents }))
        .send()
        .await
        .map_err(err)?;
    let status = r.status();
    let v: Value = r.json().await.map_err(err)?;
    if !status.is_success() {
        return Err(api_err(status, &v));
    }
    let text = v["candidates"][0]["content"]["parts"]
        .as_array()
        .map(|p| p.iter().filter_map(|x| x["text"].as_str()).collect::<Vec<_>>().join(""))
        .unwrap_or_default();
    if text.is_empty() && v["promptFeedback"]["blockReason"].is_string() {
        return Err("Requête refusée par le modèle.".into());
    }
    Ok(text)
}

/// Échange / rafraîchissement OAuth2 Discord (vocal RPC).
pub async fn discord_oauth(form: Vec<(&'static str, String)>) -> Result<Value, String> {
    let r = client()
        .post("https://discord.com/api/oauth2/token")
        .form(&form)
        .send()
        .await
        .map_err(err)?;
    let ok = r.status().is_success();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if !ok || v["access_token"].as_str().is_none() {
        let m = v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("refusé par Discord");
        return Err(m.to_string());
    }
    Ok(v)
}
