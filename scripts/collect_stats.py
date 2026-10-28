#!/usr/bin/env python3
"""Snapshot GitHub download counts and traffic into CSV files.

Run daily by .github/workflows/stats.yml, which keeps the files on the
`stats` branch. GitHub only keeps traffic for 14 days and only shows a
running total for downloads, so these files are the history.

  python3 scripts/collect_stats.py OUT_DIR

Environment: GITHUB_REPOSITORY (owner/repo), GITHUB_TOKEN (downloads),
STATS_TOKEN (optional; needed for traffic, see docs/RELEASING.md).
"""
import csv
import datetime as dt
import json
import os
import sys
import urllib.error
import urllib.request

API = "https://api.github.com"


def get(path, token):
    req = urllib.request.Request(API + path, headers={"Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "keelwire-stats"})
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def read_rows(path):
    if not os.path.exists(path):
        return []
    with open(path, newline="") as f:
        return list(csv.DictReader(f))


def write_rows(path, fields, rows):
    with open(path, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        w.writerows(rows)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "stats"
    os.makedirs(out, exist_ok=True)
    repo = os.environ["GITHUB_REPOSITORY"]
    token = os.environ.get("GITHUB_TOKEN")
    traffic_token = os.environ.get("STATS_TOKEN")
    today = dt.datetime.now(dt.timezone.utc).date().isoformat()

    # Downloads: cumulative count per release file, one snapshot per day.
    releases = []
    page = 1
    while True:
        batch = get(f"/repos/{repo}/releases?per_page=100&page={page}", token)
        releases += batch
        if len(batch) < 100:
            break
        page += 1
    path = os.path.join(out, "downloads.csv")
    rows = [r for r in read_rows(path) if r["date"] != today]
    for rel in releases:
        if rel.get("draft"):
            continue
        for a in rel.get("assets", []):
            rows.append({"date": today, "tag": rel["tag_name"], "file": a["name"], "downloads": a["download_count"]})
    write_rows(path, ["date", "tag", "file", "downloads"], rows)

    # Repository facts.
    info = get(f"/repos/{repo}", token)
    path = os.path.join(out, "repo.csv")
    rows = [r for r in read_rows(path) if r["date"] != today]
    rows.append({"date": today, "stars": info.get("stargazers_count", 0), "forks": info.get("forks_count", 0), "watchers": info.get("subscribers_count", 0), "open_issues": info.get("open_issues_count", 0)})
    write_rows(path, ["date", "stars", "forks", "watchers", "open_issues"], rows)

    # Traffic (needs a token with access to repository traffic).
    traffic = "skipped: add a STATS_TOKEN secret to collect views and clones"
    if traffic_token:
        try:
            for kind in ("views", "clones"):
                data = get(f"/repos/{repo}/traffic/{kind}", traffic_token)
                path = os.path.join(out, f"{kind}.csv")
                by_day = {r["date"]: r for r in read_rows(path)}
                for d in data.get(kind, []):
                    day = d["timestamp"][:10]
                    by_day[day] = {"date": day, "count": d["count"], "unique": d["uniques"]}
                write_rows(path, ["date", "count", "unique"], [by_day[k] for k in sorted(by_day)])
            for kind, key in (("popular/referrers", "referrer"), ("popular/paths", "path")):
                data = get(f"/repos/{repo}/traffic/{kind}", traffic_token)
                path = os.path.join(out, "referrers.csv" if key == "referrer" else "paths.csv")
                rows = [r for r in read_rows(path) if r["date"] != today]
                for d in data:
                    rows.append({"date": today, key: d[key], "count_14d": d["count"], "unique_14d": d["uniques"]})
                write_rows(path, ["date", key, "count_14d", "unique_14d"], rows)
            traffic = "collected"
        except urllib.error.HTTPError as e:
            traffic = f"failed: HTTP {e.code} (check that STATS_TOKEN can read this repository's traffic)"

    # Human-readable summary of the latest numbers.
    latest = [r for r in read_rows(os.path.join(out, "downloads.csv")) if r["date"] == today]
    total = sum(int(r["downloads"]) for r in latest)
    lines = [f"# Keelwire statistics ({today})", "", f"Total downloads, all releases: **{total:,}**", ""]
    if latest:
        lines += ["| Release | File | Downloads |", "|---|---|---|"]
        lines += [f"| {r['tag']} | {r['file']} | {int(r['downloads']):,} |" for r in sorted(latest, key=lambda r: (r["tag"], -int(r["downloads"])))]
    else:
        lines.append("No published releases yet.")
    lines += ["", f"Stars {info.get('stargazers_count', 0)} · forks {info.get('forks_count', 0)} · watchers {info.get('subscribers_count', 0)}", "", f"Traffic: {traffic}.", "", "History: downloads.csv (daily cumulative counts), repo.csv, views.csv, clones.csv, referrers.csv, paths.csv."]
    with open(os.path.join(out, "README.md"), "w") as f:
        f.write("\n".join(lines) + "\n")
    print(f"downloads total {total}; traffic {traffic}")


if __name__ == "__main__":
    main()
