"""Per-user memory for Nova — a persistent, category-tagged fact store the bot loads
at session start and grows via the remember/recall/forget tools.

One fact per entry: ``{text, category, created_at}``, category ∈
``preference | person | place | routine | project``. JSON-file backend (one file per
user) behind a small interface so a later SQLite swap is transparent.

Identity: keyed by ``user_id``. Until ``secure-bot-endpoint`` supplies real per-user
ids, the caller passes a single dev key (``MEMORY_USER_ID`` env, default ``"default"``).
Cross-user isolation is by file — two ids never share a store, and the id is
filesystem-sanitised so it can't escape the store dir.
"""

import json
import os
import re
import time

from loguru import logger

CATEGORIES = ("preference", "person", "place", "routine", "project")
# Injection ordering: identity-anchoring facts first, then tastes / ongoing work.
_ORDER = {"person": 0, "place": 1, "routine": 2, "preference": 3, "project": 4}


def _dir():
    d = os.getenv("MEMORY_DIR") or os.path.join(os.path.dirname(__file__), "memory_store")
    os.makedirs(d, exist_ok=True)
    return d


def _safe(user_id):
    """Filesystem-safe store key — a hostile user id can't traverse out of the dir."""
    return re.sub(r"[^A-Za-z0-9_.-]", "_", (user_id or "default").strip()) or "default"


def _path(user_id):
    return os.path.join(_dir(), f"{_safe(user_id)}.json")


def _load(user_id):
    try:
        with open(_path(user_id), "r", encoding="utf-8") as f:
            data = json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        data = {}
    data.setdefault("facts", [])
    data.setdefault("summary", "")
    data.setdefault("summary_at", 0)
    return data


def _save(user_id, data):
    # Write-then-rename so a crash mid-write can't corrupt the store.
    path = _path(user_id)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    os.replace(tmp, path)


def _norm(s):
    return re.sub(r"\s+", " ", (s or "").strip().lower())


def add(user_id, text, category="preference"):
    """Store a fact. A near-identical existing fact is replaced so the newest wins
    (timestamps break ties on retrieval). Returns the stored fact, or None if empty."""
    text = (text or "").strip()
    if not text:
        return None
    if category not in CATEGORIES:
        category = "preference"
    data = _load(user_id)
    key = _norm(text)
    data["facts"] = [f for f in data["facts"] if _norm(f["text"]) != key]
    fact = {"text": text, "category": category, "created_at": time.time()}
    data["facts"].append(fact)
    _save(user_id, data)
    logger.info(f"[memory:{_safe(user_id)}] +({category}) {text}")
    return fact


def query(user_id, text, limit=8):
    """Keyword/substring match over fact text + category, best match first. An empty
    query returns the most-recent facts."""
    facts = _load(user_id)["facts"]
    q = _norm(text)
    if not q:
        return sorted(facts, key=lambda f: -f["created_at"])[:limit]
    terms = [t for t in re.split(r"\W+", q) if len(t) > 2]

    def score(f):
        hay = _norm(f["text"]) + " " + f["category"]
        s = sum(1 for t in terms if t in hay)
        if q in hay:
            s += 3
        return s

    hits = [f for f in facts if score(f) > 0]
    hits.sort(key=lambda f: (-score(f), -f["created_at"]))
    return hits[:limit]


def remove(user_id, match):
    """Lenient removal: drop facts whose text contains the match, or vice-versa.
    Returns the removed facts (so the caller can confirm what went)."""
    m = _norm(match)
    if not m:
        return []
    data = _load(user_id)
    removed, kept = [], []
    for f in data["facts"]:
        t = _norm(f["text"])
        if m in t or t in m:
            removed.append(f)
        else:
            kept.append(f)
    if removed:
        data["facts"] = kept
        _save(user_id, data)
        logger.info(f"[memory:{_safe(user_id)}] -{len(removed)} matching '{match}'")
    return removed


def all_facts(user_id):
    return _load(user_id)["facts"]


def context_block(user_id, budget_chars=1400):
    """The memory text prepended to Nova's system prompt at session start. Empty store
    → "" (behaviour identical to no memory). Budgeted so a growing store can't blow up
    first-token latency: identity-anchoring facts first, then by recency, under a cap."""
    data = _load(user_id)
    facts = data["facts"]
    summary = (data.get("summary") or "").strip()
    if not facts and not summary:
        return ""
    facts_sorted = sorted(facts, key=lambda f: (_ORDER.get(f["category"], 5), -f["created_at"]))
    lines, used = [], 0
    for f in facts_sorted:
        line = f"- ({f['category']}) {f['text']}"
        if used + len(line) + 1 > budget_chars:
            break
        lines.append(line)
        used += len(line) + 1
    parts = []
    if lines:
        parts.append("Known facts about the user (from past conversations):\n" + "\n".join(lines))
    if summary:
        parts.append("Where you left off last session:\n" + summary)
    return "\n\n".join(parts)


def get_summary(user_id):
    return _load(user_id).get("summary", "")


def set_summary(user_id, text, max_chars=800):
    """Persist a short rolling recap for cross-session continuity (capped, overwrites)."""
    data = _load(user_id)
    data["summary"] = (text or "").strip()[:max_chars]
    data["summary_at"] = time.time()
    _save(user_id, data)
