import asyncio
import os
from contextlib import asynccontextmanager
from datetime import UTC, datetime, timedelta
from typing import Annotated, Any

import httpx
from fastapi import Depends, FastAPI, Response
from fastapi.requests import Request
from fastapi.responses import PlainTextResponse
from icalendar import Calendar, Event, vDate
from pydantic import BaseModel

unset_variables = []

SONARR_API_URL = os.getenv("SONARR_API_URL")
if not SONARR_API_URL:
    unset_variables.append("SONARR_API_URL")

SONARR_API_KEY = os.getenv("SONARR_API_KEY")
if not SONARR_API_KEY:
    unset_variables.append("SONARR_API_KEY")

RADARR_API_URL = os.getenv("RADARR_API_URL")
if not RADARR_API_URL:
    unset_variables.append("RADARR_API_URL")

RADARR_API_KEY = os.getenv("RADARR_API_KEY")
if not RADARR_API_KEY:
    unset_variables.append("RADARR_API_KEY")

if unset_variables:
    raise RuntimeError(f"Missing required environment variables: {', '.join(unset_variables)}")


@asynccontextmanager
async def lifespan(app: FastAPI):
    app.state.httpx = httpx.AsyncClient(
        http2=True,
        timeout=httpx.Timeout(10.0, connect=5.0),
        limits=httpx.Limits(max_connections=200, max_keepalive_connections=100),
    )
    try:
        yield
    finally:
        await app.state.httpx.aclose()


app = FastAPI(title="Calendarr", lifespan=lifespan, version="1.0.0")


def format_minutes_to_hm(minutes: int) -> str:
    """Convert total minutes into a compact string like '1h20m'."""
    minutes = max(minutes, 0)

    hours, mins = divmod(minutes, 60)

    if hours and mins:
        return f"{hours}h{mins}m"
    if hours:
        return f"{hours}h"
    return f"{mins}m"


def get_httpx_client(request: Request) -> httpx.AsyncClient:
    if not hasattr(request.app.state, "httpx"):
        raise RuntimeError("HTTP client not initialized")
    return app.state.httpx


def dump_calendar(cal: Calendar) -> str:
    return cal.to_ical().decode("utf-8").replace("\r\n", "\n").strip()


class MovieRelease(BaseModel):
    title: str
    releaseDate: datetime
    overview: str | None
    posterUrl: str | None
    imdb: str | None
    runtime: str | None

    def __repr__(self) -> str:
        return f"<MovieRelease title={self.title!r} releaseDate={self.releaseDate!r} imdb={self.imdb!r} runtime={self.runtime!r}>"

    def get_event(self) -> Event:
        event = Event()
        title = f"🎬 {self.title}"
        if self.runtime:
            title += f" ({self.runtime})"
        event.add("summary", title)
        event.add("dtstart", vDate(self.releaseDate))
        event.add("dtend", vDate(self.releaseDate + timedelta(days=1)))

        description = self.overview or ""
        if self.imdb:
            description += f"\n{self.imdb}"
        event.add("description", description)

        event.add("uid", f"{self.imdb or self.title}-{self.releaseDate.isoformat()}")
        return event


class SeriesRelease(BaseModel):
    series_title: str
    episode_title: str
    season_num: int
    episode_num: int
    airdate_utc: datetime
    overview: str | None
    posterUrl: str | None
    imdb: str | None
    runtime: str | None

    def __repr__(self) -> str:
        return f"<SeriesRelease title={self.series_title}-{self.episode_title} releaseDate={self.airdate_utc!r} imdb={self.imdb!r} runtime={self.runtime!r}>"

    def get_event(self) -> Event:
        event = Event()
        title = f"📺 {self.series_title} S{self.season_num:02d}E{self.episode_num:02d} - {self.episode_title}"
        if self.runtime:
            title += f" ({self.runtime})"
        event.add("summary", title)
        event.add("dtstart", vDate(self.airdate_utc))
        event.add("dtend", vDate(self.airdate_utc + timedelta(days=1)))

        description = self.overview or ""
        if self.imdb:
            description += f"\n{self.imdb}"
        event.add("description", description)

        event.add("uid", f"{self.episode_title or self.imdb}-{self.season_num}-{self.episode_num}")
        return event


def pick_image_url(images: list[dict[str, str]], prefer: str = "poster") -> str | None:
    # 1) try preferred coverType (e.g., "poster")
    url = next(
        (
            img["remoteUrl"]
            for img in images
            if img.get("remoteUrl") and str(img.get("coverType")).lower() == prefer.lower()
        ),
        None,
    )
    # 2) otherwise, first available url
    return url or next((img["remoteUrl"] for img in images if img.get("remoteUrl")), None)


async def fetch_radarr_events(client: httpx.AsyncClient) -> list[Event]:
    start_iso = (
        (datetime.now(UTC) - timedelta(days=15))
        .isoformat(timespec="milliseconds")
        .replace("+00:00", "Z")
    )
    end_iso = (
        (datetime.now(UTC) + timedelta(days=90))
        .isoformat(timespec="milliseconds")
        .replace("+00:00", "Z")
    )

    url = RADARR_API_URL.rstrip("/") + "/api/v3/calendar"
    api_key = RADARR_API_KEY
    result = await client.get(
        url,
        params={
            "apikey": api_key,
            "unmonitored": "false",
            "start": start_iso,
            "end": end_iso,
        },
    )

    events = []
    for movie_release in result.json():
        movie = MovieRelease(
            title=movie_release.get("title"),
            releaseDate=movie_release.get("releaseDate"),
            overview=movie_release.get("overview"),
            posterUrl=pick_image_url(images) if (images := movie_release.get("images")) else None,
            imdb=f"https://www.imdb.com/title/{movie_release.get('imdbId')}",
            runtime=format_minutes_to_hm(runtime)
            if (runtime := movie_release.get("runtime"))
            else None,
        )
        events.append(movie.get_event())

    return events


async def fetch_sonarr_events(client: httpx.AsyncClient) -> list[Event]:
    start_iso = (
        (datetime.now(UTC) - timedelta(days=15))
        .isoformat(timespec="milliseconds")
        .replace("+00:00", "Z")
    )
    end_iso = (
        (datetime.now(UTC) + timedelta(days=90))
        .isoformat(timespec="milliseconds")
        .replace("+00:00", "Z")
    )

    url = SONARR_API_URL.rstrip("/") + "/api/v3/calendar"
    api_key = SONARR_API_KEY
    result = await client.get(
        url,
        params={
            "apikey": api_key,
            "unmonitored": "false",
            "start": start_iso,
            "end": end_iso,
            "includeEpisodeImages": "true",
            "IncludeSeries": "true",
        },
    )

    events = []
    for series_release in result.json():
        movie = SeriesRelease(
            episode_title=series_release.get("title"),
            series_title=series_release.get("series", {}).get("title", "Unknown Series"),
            season_num=series_release.get("seasonNumber", 0),
            episode_num=series_release.get("episodeNumber", 0),
            airdate_utc=series_release.get("airDateUtc"),
            overview=series_release.get("overview"),
            posterUrl=pick_image_url(images) if (images := series_release.get("images")) else None,
            imdb=f"https://www.imdb.com/title/{series_release.get('series', {}).get('imdbId')}",
            runtime=format_minutes_to_hm(runtime)
            if (runtime := series_release.get("runtime"))
            else None,
        )
        events.append(movie.get_event())

    return events


@app.get("/radarr.ics", response_class=PlainTextResponse)
async def radarr_calendar(
    httpx_client: Annotated[httpx.AsyncClient, Depends(get_httpx_client)],
) -> Response:
    radarr_events = await fetch_radarr_events(httpx_client)
    radarr_events.sort(key=lambda ev: ev.get("dtstart").dt)

    cal = Calendar()
    for event in radarr_events:
        cal.add_component(event)
    return Response(content=dump_calendar(cal), media_type="text/calendar; charset=utf-8")


@app.get("/sonarr.ics", response_class=PlainTextResponse)
async def sonarr_calendar(
    httpx_client: Annotated[httpx.AsyncClient, Depends(get_httpx_client)],
) -> Response:
    sonarr_events = await fetch_sonarr_events(httpx_client)
    sonarr_events.sort(key=lambda ev: ev.get("dtstart").dt)

    cal = Calendar()
    for event in sonarr_events:
        cal.add_component(event)
    return Response(content=dump_calendar(cal), media_type="text/calendar; charset=utf-8")


@app.get("/merged.ics", response_class=PlainTextResponse)
async def merged_calendar(
    httpx_client: Annotated[httpx.AsyncClient, Depends(get_httpx_client)],
) -> Response:
    sonarr, radarr = await asyncio.gather(
        fetch_sonarr_events(httpx_client),
        fetch_radarr_events(httpx_client),
    )
    merged = [*sonarr, *radarr]
    merged.sort(key=lambda ev: ev.get("dtstart").dt)

    cal = Calendar()
    for event in merged:
        cal.add_component(event)
    return Response(content=dump_calendar(cal), media_type="text/calendar; charset=utf-8")


@app.get("/")
def root() -> dict[str, Any]:
    return {
        "ok": True,
        "endpoints": [
            "/sonarr.ics",
            "/radarr.ics",
            "/merged.ics",
        ],
        "notes": "Subscribe to these URLs from your calendar app.",
    }
