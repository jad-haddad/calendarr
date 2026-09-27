use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{DateTime, FixedOffset, NaiveDate, SecondsFormat, Utc};
use icalendar::{Calendar, Component, Event, EventLike, Property};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

const PORT: u16 = 8383;

#[derive(Clone)]
struct Config {
    sonarr_api_url: String,
    sonarr_api_key: String,
    radarr_api_url: String,
    radarr_api_key: String,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let mut missing = Vec::new();

        let sonarr_api_url = env_or_missing("SONARR_API_URL", &mut missing);
        let sonarr_api_key = env_or_missing("SONARR_API_KEY", &mut missing);
        let radarr_api_url = env_or_missing("RADARR_API_URL", &mut missing);
        let radarr_api_key = env_or_missing("RADARR_API_KEY", &mut missing);

        if !missing.is_empty() {
            return Err(format!(
                "Missing required environment variables: {}",
                missing.join(", ")
            ));
        }

        Ok(Self {
            sonarr_api_url: sonarr_api_url.unwrap(),
            sonarr_api_key: sonarr_api_key.unwrap(),
            radarr_api_url: radarr_api_url.unwrap(),
            radarr_api_key: radarr_api_key.unwrap(),
        })
    }
}

fn env_or_missing(name: &str, missing: &mut Vec<String>) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => {
            missing.push(name.to_string());
            None
        }
    }
}

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    config: Config,
}

fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .pool_max_idle_per_host(100)
        .build()
        .expect("failed to build http client")
}

#[derive(Debug, Error)]
#[error("{0}")]
struct AppError(String);

impl From<reqwest::Error> for AppError {
    fn from(value: reqwest::Error) -> Self {
        Self(value.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.0).into_response()
    }
}

fn required<T>(value: Option<T>, field: &str) -> Result<T, AppError> {
    value.ok_or_else(|| AppError(format!("missing required field: {field}")))
}

fn format_minutes_to_hm(minutes: i64) -> String {
    let minutes = minutes.max(0);
    let (hours, mins) = (minutes / 60, minutes % 60);

    if hours != 0 && mins != 0 {
        return format!("{hours}h{mins}m");
    }
    if hours != 0 {
        return format!("{hours}h");
    }
    format!("{mins}m")
}

fn dump_calendar(cal: &Calendar) -> String {
    cal.to_string().replace("\r\n", "\n").trim().to_string()
}

fn calendar_response(content: String) -> Response {
    (
        [(header::CONTENT_TYPE, "text/calendar; charset=utf-8")],
        content,
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct Image {
    #[serde(rename = "remoteUrl")]
    remote_url: Option<String>,
    #[serde(rename = "coverType")]
    cover_type: Option<String>,
}

fn pick_image_url(images: &[Image], prefer: &str) -> Option<String> {
    let preferred = images.iter().find(|img| {
        img.remote_url.is_some()
            && img
                .cover_type
                .as_deref()
                .is_some_and(|c| c.eq_ignore_ascii_case(prefer))
    });
    preferred
        .or_else(|| images.iter().find(|img| img.remote_url.is_some()))
        .and_then(|img| img.remote_url.clone())
}

#[derive(Debug, Deserialize)]
struct RadarrItem {
    title: Option<String>,
    #[serde(rename = "releaseDate")]
    release_date: Option<String>,
    overview: Option<String>,
    #[serde(rename = "imdbId")]
    imdb_id: Option<String>,
    runtime: Option<i64>,
    images: Option<Vec<Image>>,
}

struct MovieRelease {
    title: String,
    release_date: DateTime<FixedOffset>,
    overview: Option<String>,
    #[allow(dead_code)]
    poster_url: Option<String>,
    imdb: String,
    runtime: Option<String>,
}

impl MovieRelease {
    fn get_event(&self) -> (NaiveDate, Event) {
        let mut title = format!("🎬 {}", self.title);
        if let Some(runtime) = &self.runtime {
            title += &format!(" ({runtime})");
        }

        let start = self.release_date.date_naive();
        let end = start + chrono::Duration::days(1);

        let mut description = self.overview.clone().unwrap_or_default();
        description += &format!("\n{}", self.imdb);

        let uid = format!("{}-{}", self.imdb, self.release_date.to_rfc3339());

        let mut event = Event::new();
        event.summary(&title);
        event.all_day(start);
        let mut dtend = Property::new("DTEND", end.format("%Y%m%d").to_string());
        dtend.add_parameter("VALUE", "DATE");
        event.append_property(dtend);
        event.uid(&uid);
        event.description(&description);

        (start, event)
    }
}

#[derive(Debug, Deserialize)]
struct Series {
    title: Option<String>,
    #[serde(rename = "imdbId")]
    imdb_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SonarrItem {
    title: Option<String>,
    series: Option<Series>,
    #[serde(rename = "seasonNumber")]
    season_number: Option<i64>,
    #[serde(rename = "episodeNumber")]
    episode_number: Option<i64>,
    #[serde(rename = "airDateUtc")]
    air_date_utc: Option<String>,
    overview: Option<String>,
    runtime: Option<i64>,
    images: Option<Vec<Image>>,
}

struct SeriesRelease {
    series_title: String,
    episode_title: String,
    season_num: i64,
    episode_num: i64,
    airdate_utc: DateTime<FixedOffset>,
    overview: Option<String>,
    #[allow(dead_code)]
    poster_url: Option<String>,
    imdb: String,
    runtime: Option<String>,
}

impl SeriesRelease {
    fn get_event(&self) -> (NaiveDate, Event) {
        let mut title = format!(
            "📺 {} S{:02}E{:02} - {}",
            self.series_title, self.season_num, self.episode_num, self.episode_title
        );
        if let Some(runtime) = &self.runtime {
            title += &format!(" ({runtime})");
        }

        let start = self.airdate_utc.date_naive();
        let end = start + chrono::Duration::days(1);

        let mut description = self.overview.clone().unwrap_or_default();
        description += &format!("\n{}", self.imdb);

        let uid_prefix = if self.episode_title.is_empty() {
            self.imdb.clone()
        } else {
            self.episode_title.clone()
        };
        let uid = format!("{uid_prefix}-{}-{}", self.season_num, self.episode_num);

        let mut event = Event::new();
        event.summary(&title);
        event.all_day(start);
        let mut dtend = Property::new("DTEND", end.format("%Y%m%d").to_string());
        dtend.add_parameter("VALUE", "DATE");
        event.append_property(dtend);
        event.uid(&uid);
        event.description(&description);

        (start, event)
    }
}

fn window() -> (String, String) {
    let now = Utc::now();
    let start = (now - chrono::Duration::days(15)).to_rfc3339_opts(SecondsFormat::Millis, true);
    let end = (now + chrono::Duration::days(90)).to_rfc3339_opts(SecondsFormat::Millis, true);
    (start, end)
}

fn parse_datetime(value: &str) -> Result<DateTime<FixedOffset>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map_err(|e| AppError(format!("invalid datetime {value:?}: {e}")))
}

async fn fetch_radarr_events(state: &AppState) -> Result<Vec<(NaiveDate, Event)>, AppError> {
    let (start, end) = window();
    let url = format!(
        "{}/api/v3/calendar",
        state.config.radarr_api_url.trim_end_matches('/')
    );

    let result = state
        .client
        .get(url)
        .query(&[
            ("apikey", state.config.radarr_api_key.as_str()),
            ("unmonitored", "false"),
            ("start", start.as_str()),
            ("end", end.as_str()),
        ])
        .send()
        .await?
        .json::<Vec<RadarrItem>>()
        .await?;

    let mut events = Vec::with_capacity(result.len());
    for item in result {
        let release_date = required(item.release_date, "releaseDate")?;
        let images = item.images.unwrap_or_default();
        let release = MovieRelease {
            title: required(item.title, "title")?,
            release_date: parse_datetime(&release_date)?,
            overview: item.overview,
            poster_url: Some(pick_image_url(&images, "poster")).flatten(),
            imdb: format!(
                "https://www.imdb.com/title/{}",
                item.imdb_id.unwrap_or_else(|| "None".to_string())
            ),
            runtime: item.runtime.filter(|r| *r != 0).map(format_minutes_to_hm),
        };
        events.push(release.get_event());
    }

    Ok(events)
}

async fn fetch_sonarr_events(state: &AppState) -> Result<Vec<(NaiveDate, Event)>, AppError> {
    let (start, end) = window();
    let url = format!(
        "{}/api/v3/calendar",
        state.config.sonarr_api_url.trim_end_matches('/')
    );

    let result = state
        .client
        .get(url)
        .query(&[
            ("apikey", state.config.sonarr_api_key.as_str()),
            ("unmonitored", "false"),
            ("start", start.as_str()),
            ("end", end.as_str()),
            ("includeEpisodeImages", "true"),
            ("IncludeSeries", "true"),
        ])
        .send()
        .await?
        .json::<Vec<SonarrItem>>()
        .await?;

    let mut events = Vec::with_capacity(result.len());
    for item in result {
        let series = item.series.unwrap_or(Series {
            title: None,
            imdb_id: None,
        });
        let air_date_utc = required(item.air_date_utc, "airDateUtc")?;
        let images = item.images.unwrap_or_default();
        let release = SeriesRelease {
            series_title: series.title.unwrap_or_else(|| "Unknown Series".to_string()),
            episode_title: required(item.title, "title")?,
            season_num: item.season_number.unwrap_or(0),
            episode_num: item.episode_number.unwrap_or(0),
            airdate_utc: parse_datetime(&air_date_utc)?,
            overview: item.overview,
            poster_url: Some(pick_image_url(&images, "poster")).flatten(),
            imdb: format!(
                "https://www.imdb.com/title/{}",
                series.imdb_id.unwrap_or_else(|| "None".to_string())
            ),
            runtime: item.runtime.filter(|r| *r != 0).map(format_minutes_to_hm),
        };
        events.push(release.get_event());
    }

    Ok(events)
}

fn sort_and_build(mut events: Vec<(NaiveDate, Event)>) -> Calendar {
    events.sort_by_key(|(date, _)| *date);
    let mut cal = Calendar::new();
    for (_, event) in events {
        cal.push(event);
    }
    cal
}

async fn radarr_calendar(State(state): State<AppState>) -> Result<Response, AppError> {
    let events = fetch_radarr_events(&state).await?;
    Ok(calendar_response(dump_calendar(&sort_and_build(events))))
}

async fn sonarr_calendar(State(state): State<AppState>) -> Result<Response, AppError> {
    let events = fetch_sonarr_events(&state).await?;
    Ok(calendar_response(dump_calendar(&sort_and_build(events))))
}

async fn merged_calendar(State(state): State<AppState>) -> Result<Response, AppError> {
    let (sonarr, radarr) =
        tokio::try_join!(fetch_sonarr_events(&state), fetch_radarr_events(&state))?;
    let events = [sonarr, radarr].concat();
    Ok(calendar_response(dump_calendar(&sort_and_build(events))))
}

async fn root() -> Json<Value> {
    Json(json!({
        "ok": true,
        "endpoints": [
            "/sonarr.ics",
            "/radarr.ics",
            "/merged.ics",
        ],
        "notes": "Subscribe to these URLs from your calendar app.",
    }))
}

#[tokio::main]
async fn main() {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    let state = AppState {
        client: build_client(),
        config,
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/radarr.ics", get(radarr_calendar))
        .route("/sonarr.ics", get(sonarr_calendar))
        .route("/merged.ics", get(merged_calendar))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], PORT));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");
    println!("listening on {addr}");
    axum::serve(listener, app).await.expect("server error");
}
