#!/usr/bin/env python3
"""Fake Xtream Codes provider for developing itele without a real account.

Serves the player API (account, live categories, live streams), an XMLTV
guide, SVG channel logos, and local video files as live streams. Login:
demo / demo.

    python3 tools/fake-provider.py --media clip1.ts clip2.mp4

Channel N streams media file N modulo the number of files, looped and paced
to real time (when ffprobe can read its duration), like a live source.
"""

import argparse
import json
import os
import random
import subprocess
import time
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
        parts = url.path.strip("/").split("/")
        if len(parts) == 4 and parts[0] == "live" and parts[1:3] == [USER, PASSWORD] and self.media:
            sid = int(parts[3].split(".")[0])
            return self.stream(self.media[sid % len(self.media)])
        self.send_error(404)

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
RATES = {}


def byte_rate(path):
    """Bytes per second of real-time playback, or None without ffprobe."""
    if path not in RATES:
        try:
            out = subprocess.run(
                ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path],
                capture_output=True, text=True, timeout=10,
            ).stdout.strip()
            RATES[path] = os.path.getsize(path) / float(out)
        except (OSError, ValueError, subprocess.SubprocessError):
            RATES[path] = None
    return RATES[path]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", type=int, default=8089)
    parser.add_argument("--media", nargs="*", default=[], help="video files to serve as channels")
    parser.add_argument("--prefix", default="", help="prefix for channel names, to tell providers apart")
    args = parser.parse_args()
    Handler.host = f"http://127.0.0.1:{args.port}"
    Handler.categories, Handler.streams = build_catalog(Handler.host, args.prefix)
    Handler.media = args.media
    print(f"Server: {Handler.host}  username: {USER}  password: {PASSWORD}")
    print(f"{len(Handler.streams)} channels in {len(Handler.categories)} groups, {len(args.media)} media files")
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
