# calendarr

A tiny service that turns your Sonarr and Radarr calendars into subscribable
iCalendar (`.ics`) feeds, so movie and episode release dates show up in any
calendar app (Google Calendar, Apple Calendar, Thunderbird, ...).

It is written in Rust and ships as a ~6 MB static `scratch` image.

## Endpoints

| Route | Description |
| --- | --- |
| `GET /` | JSON metadata (health/info) |
| `GET /radarr.ics` | Upcoming Radarr movie releases |
| `GET /sonarr.ics` | Upcoming Sonarr episode releases |
| `GET /merged.ics` | Both, merged and sorted by air/release date |

Every feed covers a window of 15 days in the past to 90 days in the future.
Subscribe to the feed URL from your calendar app and it will keep itself updated.

## Configuration

The following environment variables are required; the service refuses to start
if any are missing.

| Variable | Example |
| --- | --- |
| `SONARR_API_URL` | `https://sonarr.example.com` |
| `SONARR_API_KEY` | `your-sonarr-api-key` |
| `RADARR_API_URL` | `https://radarr.example.com` |
| `RADARR_API_KEY` | `your-radarr-api-key` |

## Running

### Docker

```sh
docker run -d --name calendarr -p 8383:8383 \
  -e SONARR_API_URL=https://sonarr.example.com \
  -e SONARR_API_KEY=your-sonarr-api-key \
  -e RADARR_API_URL=https://radarr.example.com \
  -e RADARR_API_KEY=your-radarr-api-key \
  ghcr.io/jad-haddad/calendarr:latest
```

Feeds are then available at `http://localhost:8383/merged.ics`.

### Docker Compose

Copy the `.env.example` values into a `.env` file, then:

```sh
docker compose up -d
```

### From source

```sh
cargo run --release
# listens on 0.0.0.0:8383
```

## Image

- Registry: `ghcr.io/jad-haddad/calendarr`
- Tags: `latest`, plus `X.Y.Z`, `X.Y`, `X` for each release
- Architectures: `linux/amd64`, `linux/arm64`
- Listens on port `8383` as a non-root user
