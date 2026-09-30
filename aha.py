import argparse
import json
import os
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone

from curl_cffi import requests as curl_requests

REPO_ROOT = os.path.dirname(os.path.abspath(__file__))
TRANSCRIBE_SCRIPT = os.path.join(REPO_ROOT, "transcribe_vod_fasterwhisper.py")
DEFAULT_FORMAT = "360p"
DEFAULT_CONCURRENT_FRAGMENTS = 10
VIDEO_EXTENSIONS = (".mp4", ".mkv", ".webm", ".ts", ".mov", ".flv")

GQL_URL = "https://gql.twitch.tv/gql"
ANONYMOUS_CLIENT_ID = "ue6666qo983tsx6so1t0vnawi233wa"
VIDEO_TOWER_OPERATION = "FilterableVideoTower_Videos"
# Twitch rotates these persisted-query hashes periodically; if a run returns
# 0 videos for a channel that clearly has VODs, update this (same fix pattern
# as yt-dlp's twitch extractor, e.g. PR #15008).
VIDEO_TOWER_HASH = "67004f7881e65c297936f32c75246470629557a393788fb5a69d6d9a25a8fd5f"

# VODs become undownloadable ~60 days after the broadcast (Twitch retention).
VOD_RETENTION_DAYS = 60

BROADCAST_TYPES = {
    "all": None,
    "archive": "ARCHIVE",
    "highlight": "HIGHLIGHT",
    "upload": "UPLOAD",
}

PAGE_SIZE = 100
FETCH_MAX_RETRIES = 5
FETCH_RETRY_DELAY = 10


def fetch_videos_page(channel, broadcast_type, cursor):
    variables = {
        "channelOwnerLogin": channel,
        "broadcastType": broadcast_type,
        "videoSort": "TIME",
        "limit": PAGE_SIZE,
        "cursor": cursor,
    }
    ops = [
        {
            "operationName": VIDEO_TOWER_OPERATION,
            "variables": variables,
            "extensions": {
                "persistedQuery": {"version": 1, "sha256Hash": VIDEO_TOWER_HASH}
            },
        }
    ]
    r = curl_requests.post(
        GQL_URL,
        json=ops,
        headers={
            "Client-ID": ANONYMOUS_CLIENT_ID,
            "Content-Type": "text/plain;charset=UTF-8",
        },
        impersonate="chrome120",
        timeout=30,
    )
    r.raise_for_status()
    payload = r.json()
    if not isinstance(payload, list) or not payload:
        raise ValueError(f"Unexpected GraphQL response: {payload!r:.200}")
    result = payload[0]
    if result.get("errors"):
        raise ValueError(f"GraphQL error: {result['errors']!r:.300}")
    user = (result.get("data") or {}).get("user")
    if user is None:
        raise ValueError(f"Channel '{channel}' does not exist on Twitch")
    return user.get("videos") or {}


def fetch_videos(channel, broadcast_type):
    videos = []
    cursor = ""
    page = 0
    while True:
        page += 1
        last_error = None
        for attempt in range(1, FETCH_MAX_RETRIES + 1):
            try:
                data = fetch_videos_page(channel, broadcast_type, cursor)
                edges = data.get("edges") or []
                videos.extend(edge["node"] for edge in edges)
                has_next = bool((data.get("pageInfo") or {}).get("hasNextPage"))
                next_cursor = edges[-1].get("cursor") if edges else None
                print(
                    f"Fetched page {page} ({len(edges)} videos, "
                    f"total {len(videos)}, has_next={has_next})"
                )
                break
            except Exception as e:
                last_error = e
                print(f"Fetch attempt {attempt}/{FETCH_MAX_RETRIES} failed: {e}")
                if attempt == FETCH_MAX_RETRIES:
                    raise RuntimeError(
                        f"Could not fetch videos for channel '{channel}': {last_error}"
                    )
                time.sleep(FETCH_RETRY_DELAY * attempt)
        if not has_next or not next_cursor:
            break
        cursor = next_cursor

    if not videos:
        raise RuntimeError(f"API returned an empty video list for '{channel}'")
    return videos


def now_iso():
    return datetime.now(timezone.utc).astimezone().isoformat(timespec="seconds")


def extract_video(node):
    video_id = node.get("id")
    owner = node.get("owner") or {}
    game = node.get("game") or {}
    return {
        "id": video_id,
        "url": f"https://www.twitch.tv/videos/{video_id}",
        "title": node.get("title"),
        "created_at": node.get("publishedAt"),
        "duration_seconds": node.get("lengthSeconds"),
        "views": node.get("viewCount"),
        "thumbnail": node.get("previewThumbnailURL"),
        "game": game.get("slug"),
        "game_name": game.get("displayName"),
        "owner": owner.get("login"),
        "owner_display_name": owner.get("displayName"),
        "expired": None,
    }


def is_expired(record):
    created_at = record.get("created_at")
    if not created_at:
        return False
    try:
        created = datetime.fromisoformat(created_at.replace("Z", "+00:00"))
    except (ValueError, TypeError):
        return False
    return created < datetime.now(timezone.utc) - timedelta(days=VOD_RETENTION_DAYS)


def load_existing(path):
    if not os.path.exists(path):
        return {}
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (json.JSONDecodeError, OSError) as e:
        print(f"Warning: could not read existing file '{path}': {e}. Starting fresh.")
        return {}

    videos = data.get("videos") if isinstance(data, dict) else None
    if not isinstance(videos, list):
        return {}

    result = {}
    for entry in videos:
        if isinstance(entry, dict) and entry.get("id") is not None:
            result[str(entry["id"])] = entry
    return result


def merge_videos(existing, fetched):
    existing_by_id = dict(existing)
    added, updated = 0, 0

    for node in fetched:
        record = extract_video(node)
        record["expired"] = is_expired(record)
        key = str(record["id"])

        if key in existing_by_id:
            if existing_by_id[key] != record:
                existing_by_id[key] = record
                updated += 1
        else:
            existing_by_id[key] = record
            added += 1

    return list(existing_by_id.values()), added, updated


def save(path, videos, channel):
    output = {
        "channel": channel,
        "last_updated": now_iso(),
        "video_count": len(videos),
        "videos": sorted(videos, key=lambda v: v.get("created_at") or "", reverse=True),
    }

    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(output, f, indent=2, ensure_ascii=False)


def find_video_file(txt_dir, video_id):
    for ext in VIDEO_EXTENSIONS:
        path = os.path.join(txt_dir, f"{video_id}{ext}")
        if os.path.exists(path):
            return path
    return None


def ensure_transcriptions(videos, args):
    """Pipeline VODs through download (parallel) then transcribe (GPU-serial)."""
    os.makedirs(args.txt_dir, exist_ok=True)
    processed, missing, expired = [], [], []

    for video in videos:
        video_id = str(video["id"])
        txt_path = os.path.join(args.txt_dir, f"{video_id}.txt")
        if os.path.exists(txt_path) and os.path.getsize(txt_path) > 0 and not args.force:
            processed.append(video_id)
        elif video.get("expired"):
            expired.append(video_id)
        else:
            missing.append(video)

    if expired:
        print(
            f"{len(expired)} VOD(s) expired (older than {VOD_RETENTION_DAYS} days, "
            f"no longer downloadable): {expired}"
        )
    if not missing:
        print(f"All {len(processed)} VOD(s) already transcribed ({args.txt_dir}).")
        return

    print(f"{len(missing)} VOD(s) need transcription: {[str(v['id']) for v in missing]}")
    if args.check_only:
        return

    txt_dir = os.path.abspath(args.txt_dir)
    log_dir = os.path.join(REPO_ROOT, "downloaded_logs")
    os.makedirs(log_dir, exist_ok=True)

    download_pending = list(missing)
    download_pending.sort(key=lambda v: v.get("created_at") or "")
    transcribe_pending = []
    active_downloads = {}
    active_transcribes = {}
    results = {}

    def start_download(video):
        video_id = str(video["id"])
        log_path = os.path.join(log_dir, f"{video_id}.download.log")
        log_file = open(log_path, "w", encoding="utf-8")
        # Download directly with yt-dlp (no opus conversion anywhere in this
        # pipeline: transcription later runs with --skip-conversion, i.e.
        # straight from the .mp4). Previously this shelled out to
        # transcribe_vod_fasterwhisper.py with flags it no longer accepts
        # (--streamer-name/--vod-id/--format/--download-only), so every
        # download failed with exit 2.
        outtmpl = os.path.join(txt_dir, f"{video_id}.%(ext)s")
        cmd = [
            sys.executable,
            "-m",
            "yt_dlp",
            "--concurrent-fragments",
            str(args.concurrent_fragments),
            "-f",
            args.format,
            "--retries",
            "3",
            "-o",
            outtmpl,
            video["url"],
        ]
        print(f"[download ] starting {video_id}")
        proc = subprocess.Popen(cmd, stdout=log_file, stderr=subprocess.STDOUT)
        active_downloads[video_id] = (proc, log_file)

    def start_transcribe(video_id, video_file):
        log_path = os.path.join(log_dir, f"{video_id}.transcribe.log")
        log_file = open(log_path, "w", encoding="utf-8")
        cmd = [sys.executable, TRANSCRIBE_SCRIPT, video_file, "--skip-conversion"]
        print(f"[transcribe] starting {video_id}")
        proc = subprocess.Popen(cmd, stdout=log_file, stderr=subprocess.STDOUT)
        active_transcribes[video_id] = (proc, log_file)

    while download_pending and len(active_downloads) < args.download_workers:
        start_download(download_pending.pop(0))

    while active_downloads or active_transcribes or download_pending or transcribe_pending:
        time.sleep(1)

        finished_downloads = [
            vid for vid, (proc, _) in active_downloads.items() if proc.poll() is not None
        ]
        for video_id in finished_downloads:
            proc, log_file = active_downloads.pop(video_id)
            log_file.close()
            if proc.returncode != 0:
                print(f"[download ] FAILED {video_id} (exit {proc.returncode})")
                results[video_id] = False
                continue
            video_file = find_video_file(txt_dir, video_id)
            if video_file:
                print(f"[download ] done {video_id} -> {os.path.basename(video_file)}")
                transcribe_pending.append((video_id, video_file))
            else:
                print(f"[download ] FAILED {video_id}: file not found after download")
                results[video_id] = False

        while download_pending and len(active_downloads) < args.download_workers:
            start_download(download_pending.pop(0))

        finished_transcribes = [
            vid for vid, (proc, _) in active_transcribes.items() if proc.poll() is not None
        ]
        for video_id in finished_transcribes:
            proc, log_file = active_transcribes.pop(video_id)
            log_file.close()
            txt_path = os.path.join(txt_dir, f"{video_id}.txt")
            if proc.returncode == 0 and os.path.exists(txt_path) and os.path.getsize(txt_path) > 0:
                print(f"[transcribe] OK {video_id} -> {txt_path}")
                results[video_id] = True
                if not args.keep_videos:
                    video_file = find_video_file(txt_dir, video_id)
                    if video_file:
                        try:
                            os.remove(video_file)
                            print(f"[cleanup   ] deleted {os.path.basename(video_file)}")
                        except OSError as e:
                            print(f"[cleanup   ] WARN could not delete {video_file}: {e}")
            else:
                print(f"[transcribe] FAILED {video_id} (exit {proc.returncode})")
                results[video_id] = False

        while transcribe_pending and len(active_transcribes) < args.transcribe_workers:
            video_id, video_file = transcribe_pending.pop(0)
            start_transcribe(video_id, video_file)

    ok = sum(1 for ok in results.values() if ok)
    failed = len(results) - ok
    print(f"\nDone: {ok} transcribed, {failed} failed of {len(missing)} VOD(s).")


def main():
    parser = argparse.ArgumentParser(
        description="Fetch Twitch channel VODs into a JSON file (updates on re-run)"
    )
    parser.add_argument("--channel", required=True, help="Twitch channel name")
    parser.add_argument(
        "--output",
        default=None,
        help="Output JSON path (default: <channel>_twitch_vods.json, distinct from the Kick script's <channel>_vods.json)",
    )
    parser.add_argument(
        "--txt-dir",
        default=None,
        help="Directory where transcription .txt files live (default: <channel>/ under this repo)",
    )
    parser.add_argument(
        "--type",
        choices=list(BROADCAST_TYPES),
        default="all",
        help="Which videos to fetch: all, archive (past broadcasts), highlight, upload (default: all)",
    )
    parser.add_argument(
        "--format",
        default=DEFAULT_FORMAT,
        help=f"yt-dlp format selector for downloading VODs (default: {DEFAULT_FORMAT})",
    )
    parser.add_argument(
        "-N",
        "--concurrent-fragments",
        type=int,
        default=DEFAULT_CONCURRENT_FRAGMENTS,
        help=f"Concurrent HLS fragments for yt-dlp (default: {DEFAULT_CONCURRENT_FRAGMENTS})",
    )
    parser.add_argument(
        "--check-only",
        action="store_true",
        help="Only report which VODs still need transcription, do not process them.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Re-transcribe every VOD even if its .txt already exists.",
    )
    parser.add_argument(
        "--download-workers",
        type=int,
        default=2,
        help="Concurrent VOD downloads (default: 2).",
    )
    parser.add_argument(
        "--transcribe-workers",
        type=int,
        default=1,
        help="Concurrent transcriptions (default: 1; keep low to limit GPU VRAM use).",
    )
    parser.add_argument(
        "--keep-videos",
        action="store_true",
        help="Keep the downloaded .mp4 files after successful transcription (default: delete them).",
    )
    args = parser.parse_args()

    args.output = args.output or f"{args.channel}_twitch_vods.json"
    args.txt_dir = args.txt_dir or os.path.join(REPO_ROOT, args.channel)

    print(f"Fetching videos for channel '{args.channel}' (type={args.type})...")
    fetched = fetch_videos(args.channel, BROADCAST_TYPES[args.type])

    existing = load_existing(args.output)
    print(f"Loaded {len(existing)} existing video(s) from '{args.output}'")

    merged, added, updated = merge_videos(existing, fetched)
    save(args.output, merged, args.channel)
    print(f"Added {added} new, updated {updated}, total {len(merged)} videos -> '{args.output}'")

    ensure_transcriptions(merged, args)


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)
