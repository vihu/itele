#!/usr/bin/env python3
"""Fake Xtream Codes provider for developing itele without a real account.

Serves the player API (account, live TV, movies, series), an XMLTV guide,
channel logos and posters, and local video files as live channels, movies
and episodes. Login: demo / demo.

    python3 tools/fake-provider.py --media clip1.ts clip2.mp4

Channel N streams media file N modulo the number of files, looped and paced
to real time (when ffprobe can read its duration), like a live source.
Movies and episodes serve the files whole, with byte ranges for seeking.
Posters are generated (SVG and PNG) unless --posters names a folder of
images to use instead.
"""

import argparse
import json
import os
import random
import struct
import subprocess
import time
import zlib
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import parse_qs, urlparse
from xml.sax.saxutils import escape, quoteattr

USER, PASSWORD = "demo", "demo"

GROUPS = {
    "Documentary": ["Atlas Nature HD", "Halcyon Docs", "Deep Earth", "Wild Coasts TV"],
    "Sports": ["Volt Sports 1", "Volt Sports 2", "Arena 24", "Track & Field", "Ringside", "Paddock"],
    "News": ["Meridian News", "World Desk", "Nordic 24", "Capital Report", "Morning Wire"],
    "Movies": ["Cinéclub", "Northline Drama", "Midnight Reel", "Classic Picture House"],
    "Kids": ["Kidsbox", "Paper Boats", "Tiny Planet"],
    "Music": ["Tempo Music", "Late Mix", "Arcade FM TV"],
    "France": ["Rivage 1", "Brume TV", "Canal Sud", "Lumière 2", "Côte Est"],
    "United Kingdom": ["Harbour One", "Thames Live", "Highland TV", "Borders Channel"],
    "Deutschland": ["Rheinwelle", "Nordlicht", "Alpenblick HD"],
}
TITLES = [
    "The Meadow Year", "Night Shift: Owls", "Wild Coasts", "Harbour Lights", "The Evening Desk",
    "World Tonight", "Coastal League Live", "Track Cycling", "Late Mix", "Live at the Arcade",
    "Engines of the Deep", "Cities After Dark", "Pip and the Paper Boats", "Goodnight Stories",
    "Les Années Lumière", "Journal de la nuit", "Night Train to Varna", "Short Film Hour",
    "Rivers of Ice", "The Salt Road", "Morning Wire", "Capital Report", "Ringside", "Paddock Live",
    "Quiet Streets", "Moss & Pebble", "Tiny Planet", "Highland Walks", "Rheinabend", "Alpenblick",
]
VOD_GROUPS = ["Action", "Comedy", "Drama", "Family", "Documentary", "Thriller", "Français", "Deutsch", "4K"]
SERIES_GROUPS = ["Comedy", "Drama", "Kids", "Crime", "Documentary"]
WORDS = [
    "Cottontail", "Red", "Fury", "Lichtung", "Apples", "August", "Stakes", "Glide", "Second", "Thoughts",
    "Small", "Hours", "Pact", "Far", "Field", "Terrier", "Shade", "Gang", "Three", "Big", "Feelings",
    "Nutcase", "Undergrowth", "Sharp", "Ends", "Orchard", "Pantry", "Meadow", "Burrow", "Harvest",
]
PLOTS = [
    "Three woodland rodents run the most ambitious heist crew in the valley.",
    "A quiet rabbit finally has enough of the neighbours and plans something drastic.",
    "One apple, three thieves, and no plan that survives contact with gravity.",
    "A long summer in the meadow turns into a race against the first frost.",
]
COLORS = ["#1f5e3b", "#2a3cc7", "#b3261e", "#0f6e8c", "#3b3f4a", "#c2185b", "#5b6b2e", "#d35400", "#6b5ca5"]


def build_catalog(host, prefix):
    categories, streams, number = [], [], 1
    rng = random.Random(7)
    for cat_id, (group, names) in enumerate(GROUPS.items(), start=1):
        categories.append({"category_id": str(cat_id), "category_name": group, "parent_id": 0})
        for name in names:
            sid = 1000 + number
            streams.append({
                "num": number,
                "name": prefix + name,
                "stream_type": "live",
                "stream_id": sid,
                # Every third channel has no logo, to exercise the fallback tile.
                "stream_icon": "" if number % 3 == 0 else f"{host}/logo/{sid}.svg",
                "epg_channel_id": name.lower().replace(" ", "."),
                "added": "1700000000",
                "category_id": str(cat_id),
                "custom_sid": "",
                "tv_archive": rng.choice([0, 1]),
                "direct_source": "",
                "tv_archive_duration": rng.choice([3, 7]),
            })
            number += 1
    return categories, streams


def title_for(rng, n):
    words = rng.sample(WORDS, rng.choice([1, 2, 2, 3]))
    return " ".join(words) if n % 4 else "The " + " ".join(words)


def build_vod(host, prefix, count, media):
    rng = random.Random(11)
    categories = [{"category_id": str(100 + i), "category_name": g, "parent_id": 0} for i, g in enumerate(VOD_GROUPS)]
    movies = []
    for n in range(count):
        sid = 5000 + n
        year = rng.randint(1985, 2026)
        # Some names end with the year, as many providers write them.
        name = prefix + title_for(rng, n) + (f" ({year})" if n % 3 == 0 else "")
        ext = os.path.splitext(media[sid % len(media)])[1].lstrip(".") if media else "mp4"
        movies.append({
            "num": n + 1, "name": name, "stream_type": "movie", "stream_id": sid,
            # Every fifth movie has no poster, to exercise the fallback tile.
            "stream_icon": "" if n % 5 == 4 else f"{host}/poster/{sid}.{'png' if n % 2 else 'svg'}",
            "rating": str(round(rng.uniform(4.5, 8.9), 1)), "rating_5based": 0,
            "added": str(1700000000 + n * 3600), "category_id": str(100 + n % len(VOD_GROUPS)),
            "container_extension": ext, "custom_sid": "", "direct_source": "",
            "year": str(year) if n % 3 == 1 else "",
        })
    return categories, movies


def build_series(host, prefix, count):
    rng = random.Random(13)
    categories = [{"category_id": str(200 + i), "category_name": g, "parent_id": 0} for i, g in enumerate(SERIES_GROUPS)]
    shows = []
    for n in range(count):
        sid = 7000 + n
        year = rng.randint(1995, 2026)
        shows.append({
            "num": n + 1, "name": prefix + title_for(rng, n + 1), "series_id": sid,
            "cover": "" if n % 6 == 5 else f"{host}/poster/{sid}.{'png' if n % 2 else 'svg'}",
            "plot": rng.choice(PLOTS), "cast": "Ana Dimas, Teo Larsen, Margit Holm", "director": "Ida Sund",
            "genre": SERIES_GROUPS[n % len(SERIES_GROUPS)], "releaseDate": f"{year}-03-01",
            "last_modified": str(1700000000 + n * 7200), "rating": str(round(rng.uniform(5.5, 9.1), 1)),
            "rating_5based": 0, "backdrop_path": [f"{host}/backdrop/{sid}.svg"] if n % 2 == 0 else [],
            "youtube_trailer": "", "episode_run_time": rng.choice(["24", "45", "58"]),
            "category_id": str(200 + n % len(SERIES_GROUPS)),
        })
    return categories, shows


def series_info(show, host, media):
    rng = random.Random(show["series_id"])
    sid = show["series_id"]
    seasons, episodes = [], {}
    for season in range(1, rng.randint(1, 4) + 1):
        seasons.append({"season_number": season, "name": f"Season {season}", "overview": "",
                        "cover": show["cover"], "episode_count": 0, "air_date": ""})
        items = []
        for number in range(1, rng.randint(4, 10) + 1):
            eid = sid * 1000 + season * 100 + number
            path = media[eid % len(media)] if media else ""
            items.append({
                "id": str(eid), "episode_num": number, "title": title_for(rng, number), "season": season,
                "container_extension": os.path.splitext(path)[1].lstrip(".") or "mp4",
                "info": {"movie_image": f"{host}/backdrop/{eid}.svg", "plot": rng.choice(PLOTS),
                         "duration_secs": int(media_duration(path) or 0) if path else 0, "releasedate": ""},
                "custom_sid": "", "added": "1700000000", "direct_source": "",
            })
        episodes[str(season)] = items
    # Some panels send no seasons at all; the player must work from episodes.
    if sid % 5 == 0:
        seasons = []
    return {"seasons": seasons, "info": {k: v for k, v in show.items() if k not in ("num", "series_id")},
            "episodes": episodes}


def vod_info(movie, host, media):
    sid = movie["stream_id"]
    path = media[sid % len(media)] if media else ""
    rng = random.Random(sid)
    return {
        "info": {
            "movie_image": movie["stream_icon"], "backdrop_path": [f"{host}/backdrop/{sid}.svg"] if sid % 2 else [],
            "plot": rng.choice(PLOTS), "cast": "Ana Dimas, Teo Larsen", "director": "Ida Sund",
            "genre": VOD_GROUPS[int(movie["category_id"]) - 100], "releasedate": f"{rng.randint(1985, 2026)}-06-01",
            "duration_secs": int(media_duration(path) or 0) if path else 0, "rating": movie["rating"],
        },
        "movie_data": {"stream_id": sid, "name": movie["name"], "container_extension": movie["container_extension"],
                       "category_id": movie["category_id"]},
    }


def poster_svg(title, color, width=300, height=450):
    lines = title.split()
    text = "".join(
        f'<text x="24" y="{height - 40 - 34 * (len(lines) - 1 - i)}" font-family="sans-serif" font-size="30" '
        f'font-weight="800" fill="#fff">{escape(w)}</text>'
        for i, w in enumerate(lines)
    )
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">'
        f'<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{color}"/>'
        f'<stop offset="1" stop-color="#0b0c10"/></linearGradient></defs>'
        f'<rect width="{width}" height="{height}" fill="url(#g)"/>{text}</svg>'
    )


def poster_png(color, width=300, height=450):
    """A vertical gradient from `color` to near black, as a PNG."""
    r, g, b = (int(color[i:i + 2], 16) for i in (1, 3, 5))
    rows = []
    for y in range(height):
        t = y / (height - 1)
        pixel = bytes((int(r * (1 - t) + 11 * t), int(g * (1 - t) + 12 * t), int(b * (1 - t) + 16 * t)))
        rows.append(b"\x00" + pixel * width)

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(b"".join(rows))) + chunk(b"IEND", b"")


def logo_svg(name, color):
    word = name.split()[0].upper()[:8]
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="168" height="108" viewBox="0 0 168 108">'
        f'<rect width="168" height="108" rx="20" fill="{color}"/>'
        f'<text x="84" y="64" font-family="sans-serif" font-size="28" font-weight="800" '
        f'fill="#fff" text-anchor="middle">{word}</text></svg>'
    )


def xmltv(streams, now):
    """A guide from three days back to two days ahead, stable per channel."""
    start = now // 1800 * 1800 - 3 * 86400
    end = now + 2 * 86400

    def stamp(t):
        return time.strftime("%Y%m%d%H%M%S +0000", time.gmtime(t))

    out = ['<?xml version="1.0" encoding="UTF-8"?>', '<tv generator-info-name="itele fake provider">']
    for s in streams:
        out.append(f'<channel id={quoteattr(s["epg_channel_id"])}><display-name>{escape(s["name"])}</display-name></channel>')
    for s in streams:
        rng = random.Random(s["stream_id"] * 7919 + start // 86400)
        t, episode = start, 1
        while t < end:
            duration = rng.choice([30, 45, 60, 60, 90, 120]) * 60
            title = rng.choice(TITLES)
            desc = f"Episode {episode}. {title} on {s['name']}, a programme made up for testing itele."
            out.append(
                f'<programme start="{stamp(t)}" stop="{stamp(t + duration)}" channel={quoteattr(s["epg_channel_id"])}>'
                f'<title lang="en">{escape(title)}</title><desc lang="en">{escape(desc)}</desc></programme>'
            )
            t += duration
            episode += 1
    out.append("</tv>")
    return "\n".join(out)


class Handler(BaseHTTPRequestHandler):
    categories, streams, media, host = [], [], [], ""
    vod_categories, movies, series_categories, shows, posters = [], [], [], [], []

    def log_message(self, *args):
        pass

    def send(self, body, content_type="application/json"):
        data = body.encode() if isinstance(body, str) else body
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        url = urlparse(self.path)
        if url.path == "/player_api.php":
            q = {k: v[0] for k, v in parse_qs(url.query).items()}
            ok = q.get("username") == USER and q.get("password") == PASSWORD
            action = q.get("action")
            if action is None:
                info = {"auth": 0}
                if ok:
                    info = {"username": USER, "password": PASSWORD, "message": "", "auth": 1,
                            "status": "Active", "exp_date": "1799193600", "is_trial": "0",
                            "active_cons": "0", "max_connections": "2",
                            "allowed_output_formats": ["ts", "m3u8"]}
                return self.send(json.dumps({"user_info": info, "server_info": {"url": self.host}}))
            if not ok:
                return self.send("[]")
            if action == "get_live_categories":
                return self.send(json.dumps(self.categories))
            if action == "get_live_streams":
                return self.send(json.dumps(self.streams))
            if action == "get_vod_categories":
                return self.send(json.dumps(self.vod_categories))
            if action == "get_vod_streams":
                return self.send(json.dumps(self.movies))
            if action == "get_series_categories":
                return self.send(json.dumps(self.series_categories))
            if action == "get_series":
                return self.send(json.dumps(self.shows))
            if action == "get_vod_info":
                movie = next((m for m in self.movies if str(m["stream_id"]) == q.get("vod_id")), None)
                return self.send(json.dumps(vod_info(movie, self.host, self.media) if movie else {"info": [], "movie_data": []}))
            if action == "get_series_info":
                show = next((x for x in self.shows if str(x["series_id"]) == q.get("series_id")), None)
                return self.send(json.dumps(series_info(show, self.host, self.media) if show else []))
            return self.send("[]")
        if url.path == "/xmltv.php":
            q = {k: v[0] for k, v in parse_qs(url.query).items()}
            if q.get("username") == USER and q.get("password") == PASSWORD:
                return self.send(xmltv(self.streams, int(time.time())), "application/xml")
            return self.send_error(401)
        if url.path.startswith("/logo/"):
            sid = int(url.path.split("/")[-1].split(".")[0])
            stream = next((s for s in self.streams if s["stream_id"] == sid), None)
            if stream:
                return self.send(logo_svg(stream["name"], COLORS[sid % len(COLORS)]), "image/svg+xml")
        if url.path.startswith(("/poster/", "/backdrop/")):
            kind, name = url.path.strip("/").split("/")
            key, ext = name.split(".")
            color = COLORS[int(key) % len(COLORS)]
            if self.posters:
                return self.send_file(self.posters[int(key) % len(self.posters)])
            if ext == "png":
                return self.send(poster_png(color), "image/png")
            title = next((m["name"] for m in self.movies + self.shows
                          if str(m.get("stream_id", m.get("series_id"))) == key), "Episode " + key[-2:])
            size = (300, 450) if kind == "poster" else (960, 540)
            return self.send(poster_svg(title, color, *size), "image/svg+xml")
        parts = url.path.strip("/").split("/")
        if len(parts) == 4 and parts[0] in ("movie", "series") and parts[1:3] == [USER, PASSWORD] and self.media:
            sid = int(parts[3].split(".")[0])
            return self.send_file(self.media[sid % len(self.media)])
        # Catch-up: /timeshift/user/pass/minutes/YYYY-MM-DD:HH-MM/id.ts
        if len(parts) == 6 and parts[0] == "timeshift" and parts[1:3] == [USER, PASSWORD] and self.media:
            sid = int(parts[5].split(".")[0])
            return self.stream(self.media[(sid + 1) % len(self.media)])
        if len(parts) == 4 and parts[0] == "live" and parts[1:3] == [USER, PASSWORD] and self.media:
            sid = int(parts[3].split(".")[0])
            return self.stream(self.media[sid % len(self.media)])
        self.send_error(404)

    def send_file(self, path):
        """Sends a whole file, honouring a single byte range for seeking."""
        size = os.path.getsize(path)
        start, end = 0, size - 1
        wanted = self.headers.get("Range", "")
        if wanted.startswith("bytes="):
            first, _, last = wanted[6:].split(",")[0].strip().partition("-")
            if first:
                start, end = int(first), min(int(last), size - 1) if last else size - 1
            elif last:
                start = max(0, size - int(last))
            if start >= size:
                self.send_response(416)
                self.send_header("Content-Range", f"bytes */{size}")
                return self.end_headers()
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        else:
            self.send_response(200)
        types = {".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".mkv": "video/x-matroska",
                 ".mp4": "video/mp4", ".ts": "video/mp2t"}
        self.send_header("Content-Type", types.get(os.path.splitext(path)[1].lower(), "application/octet-stream"))
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        self.end_headers()
        try:
            with open(path, "rb") as f:
                f.seek(start)
                left = end - start + 1
                while left > 0 and (chunk := f.read(min(256 * 1024, left))):
                    self.wfile.write(chunk)
                    left -= len(chunk)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def stream(self, path):
        self.send_response(200)
        self.send_header("Content-Type", "video/mp2t")
        self.end_headers()
        rate = byte_rate(path)
        start, sent = time.monotonic(), 0
        try:
            while True:
                with open(path, "rb") as f:
                    while chunk := f.read(64 * 1024):
                        self.wfile.write(chunk)
                        sent += len(chunk)
                        if rate:
                            # Real time after an initial burst of BURST seconds.
                            ahead = sent / rate - (time.monotonic() - start) - BURST
                            if ahead > 0:
                                time.sleep(ahead)
        except (BrokenPipeError, ConnectionResetError):
            pass


BURST = 2.0
DURATIONS = {}


def media_duration(path):
    """Seconds of playback, or None without ffprobe."""
    if path not in DURATIONS:
        try:
            out = subprocess.run(
                ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path],
                capture_output=True, text=True, timeout=10,
            ).stdout.strip()
            DURATIONS[path] = float(out)
        except (OSError, ValueError, subprocess.SubprocessError):
            DURATIONS[path] = None
    return DURATIONS[path]


def byte_rate(path):
    """Bytes per second of real-time playback, or None without ffprobe."""
    duration = media_duration(path)
    return os.path.getsize(path) / duration if duration else None


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", type=int, default=8089)
    parser.add_argument("--media", nargs="*", default=[], help="video files to serve as channels")
    parser.add_argument("--prefix", default="", help="prefix for channel and title names, to tell providers apart")
    parser.add_argument("--movies", type=int, default=120, help="number of movies")
    parser.add_argument("--series", type=int, default=30, help="number of series")
    parser.add_argument("--posters", help="folder of .jpg/.png images to serve as posters and backdrops")
    args = parser.parse_args()
    Handler.host = f"http://127.0.0.1:{args.port}"
    Handler.categories, Handler.streams = build_catalog(Handler.host, args.prefix)
    Handler.media = args.media
    Handler.vod_categories, Handler.movies = build_vod(Handler.host, args.prefix, args.movies, args.media)
    Handler.series_categories, Handler.shows = build_series(Handler.host, args.prefix, args.series)
    if args.posters:
        Handler.posters = sorted(
            os.path.join(args.posters, f) for f in os.listdir(args.posters)
            if f.lower().endswith((".jpg", ".jpeg", ".png"))
        )
    print(f"Server: {Handler.host}  username: {USER}  password: {PASSWORD}")
    print(f"{len(Handler.streams)} channels in {len(Handler.categories)} groups, {len(args.media)} media files")
    print(f"{len(Handler.movies)} movies, {len(Handler.shows)} series")
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
