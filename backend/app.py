import os, re, io, csv, json, base64, hashlib, sqlite3, datetime as dt
from fastapi import FastAPI, UploadFile, File, Form, Header, HTTPException, Depends
from fastapi.responses import StreamingResponse, RedirectResponse
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel
from rapidfuzz import fuzz

MODEL = os.getenv("CLAUDE_MODEL", "claude-sonnet-5-5")
KEY = os.getenv("TL_KEY", "")  # shared secret for the WhatsApp bridge / phone listener
db = sqlite3.connect(os.getenv("TL_DB", "leads.db"), check_same_thread=False)
db.row_factory = sqlite3.Row
db.execute("create table if not exists leads(id integer primary key,ts text,source text,sender text,raw text,hash text,data text,cluster int,kind text)")
app = FastAPI(title="TrainLead")
_ai = None

def auth(x_key: str = Header("")):
    if KEY and x_key != KEY:
        raise HTTPException(401, "bad key")

SYS = """You extract training requirements from WhatsApp group messages posted by vendors/colleges in India.
Input: numbered messages "[i] sender: text", or an image of a chat. Today is {today}.
Return ONLY a JSON array, one object per genuine training requirement. Skip job/hiring ads, greetings, chatter and trainer-availability posts. One message may hold several requirements (e.g. several cities).
Keys: msg (int, message index; 0 for images), tech_stack (list, normalized e.g. "AngularJS","Java"), start_date, end_date (YYYY-MM-DD or null, infer year from today), duration_days (int|null), college, city, state, mode ("offline"|"online"|"hybrid"|null), audience (e.g. "6th sem Engg"), travel, food, accommodation (each "provided"|"not provided"|"unknown"), logistics_note (e.g. "TFA provided"), rate, payment_date, vendor_name, vendor_company, vendor_phone (digits with country code, else null), summary (one line)."""

def ask(content):
    global _ai
    if _ai is None:
        import anthropic
        _ai = anthropic.Anthropic()
    r = _ai.messages.create(model=MODEL, max_tokens=4000, messages=[{"role": "user", "content": content}],
                            system=SYS.replace("{today}", dt.date.today().isoformat()))
    m = re.search(r"\[.*\]", r.content[0].text, re.S)
    try:
        return json.loads(m.group(0)) if m else []
    except Exception:
        return []

norm = lambda t: re.sub(r"\W+", " ", t.lower()).strip()
KW = re.compile(r"train|workshop|program|requirement|college|fdp|batch|dates?|session|bootcamp|webinar|\bday", re.I)

def similar(a, b, ra, rb):
    s = 0
    if a.get("start_date") and a.get("start_date") == b.get("start_date"): s += 1
    loc = lambda d: (d.get("college") or d.get("city") or "").lower()
    if loc(a) and loc(b) and fuzz.token_set_ratio(loc(a), loc(b)) >= 80: s += 1
    ta, tb = {t.lower() for t in a.get("tech_stack") or []}, {t.lower() for t in b.get("tech_stack") or []}
    if ta and tb and len(ta & tb) / len(ta | tb) >= .5: s += 1
    return s >= 3 or (s == 2 and fuzz.token_set_ratio(norm(ra), norm(rb)) >= 70)

def save(source, m, d):
    key = norm(m["text"]) + "|" + str(d.get("college")) + str(d.get("city")) + str(d.get("start_date"))
    h = hashlib.sha1(key.encode()).hexdigest()
    cluster, kind = None, "new"
    for r in db.execute("select cluster,hash,data,raw from leads"):
        if r["hash"] == h: cluster, kind = r["cluster"], "exact"; break
        if similar(d, json.loads(r["data"]), m["text"], r["raw"]): cluster, kind = r["cluster"], "similar"; break
    cur = db.execute("insert into leads(ts,source,sender,raw,hash,data,cluster,kind) values(?,?,?,?,?,?,?,?)",
                     (m.get("ts") or dt.datetime.now().isoformat(timespec="minutes"), source, m.get("sender"), m["text"], h, json.dumps(d), cluster, kind))
    if cluster is None: db.execute("update leads set cluster=id where id=?", (cur.lastrowid,))
    db.commit()
    return kind

def tally(kinds):
    return {"found": len(kinds), **{k: kinds.count(k) for k in ("new", "exact", "similar")}}

def ingest(source, msgs):
    cand = [m for m in msgs if len(m["text"]) > 40 and KW.search(m["text"])]
    kinds = []
    for i in range(0, len(cand), 8):
        ch = cand[i:i + 8]
        txt = "\n\n".join(f"[{j}] {m.get('sender') or '?'}: {m['text']}" for j, m in enumerate(ch))
        for l in ask(txt):
            j = l.get("msg")
            if isinstance(j, int) and 0 <= j < len(ch): kinds.append(save(source, ch[j], l))
    return tally(kinds)

LINE = re.compile(r"^\[?(\d{1,2}[/.]\d{1,2}[/.]\d{2,4}),?\s+(\d{1,2}:\d{2}(?::\d{2})?(?:\s?[APap]\.?[Mm]\.?)?)\]?\s*-?\s*([^:]{1,60}?):\s(.*)$")
TS = re.compile(r"^\[?\d{1,2}[/.]\d{1,2}[/.]\d{2,4},?\s+\d{1,2}:\d{2}")
def parse_export(txt):
    msgs, cur = [], None
    for line in txt.replace("\u200e", "").splitlines():
        m = LINE.match(line)
        if m: cur = {"ts": f"{m[1]} {m[2]}", "sender": m[3], "text": m[4]}; msgs.append(cur)
        elif TS.match(line): cur = None  # system line, not part of a message
        elif cur: cur["text"] += "\n" + line
    return msgs

def from_export(raw, filename):
    return ingest(re.sub(r"^WhatsApp Chat (with |- )|\.txt$", "", filename), parse_export(raw.decode("utf-8", "ignore")))

def from_image(raw, ctype, source):
    b = base64.b64encode(raw).decode()
    ls = ask([{"type": "image", "source": {"type": "base64", "media_type": ctype or "image/png", "data": b}},
              {"type": "text", "text": "Extract every training requirement visible. Use msg=0."}])
    return tally([save(source, {"text": l.get("summary") or json.dumps(l)}, l) for l in ls])

class Msg(BaseModel):
    sender: str | None = None
    ts: str | None = None
    text: str
class Batch(BaseModel):
    source: str
    messages: list[Msg]

@app.post("/api/ingest/text")  # UI paste box
def api_text(b: Batch): return ingest(b.source, [m.model_dump() for m in b.messages])

@app.post("/api/ingest/messages", dependencies=[Depends(auth)])  # WhatsApp bridge + Android listener
def api_msgs(b: Batch): return ingest(b.source, [m.model_dump() for m in b.messages])

@app.post("/api/ingest/export")
def api_export(file: UploadFile = File(...)): return from_export(file.file.read(), file.filename or "export.txt")

@app.post("/api/ingest/screenshot")
def api_shot(file: UploadFile = File(...)): return from_image(file.file.read(), file.content_type, "Screenshot")

@app.post("/api/share")  # Android "Share to TrainLead" (PWA share target)
def api_share(text: str = Form(""), title: str = Form(""), file: UploadFile | None = File(None)):
    if file and file.filename:
        raw = file.file.read()
        if (file.content_type or "").startswith("image/"): from_image(raw, file.content_type, title or "Shared")
        else: from_export(raw, file.filename)
    elif text: ingest(title or "Shared", [{"text": text}])
    return RedirectResponse("/", 303)

def clusters():
    cl = {}
    for r in db.execute("select * from leads order by id desc"):
        r = dict(r); r["data"] = json.loads(r["data"]); cl.setdefault(r["cluster"], []).append(r)
    out = []
    for cid, ms in cl.items():
        best = max(ms, key=lambda r: sum(v not in (None, "", "unknown", []) for v in r["data"].values()))
        out.append({"id": cid, "data": best["data"], "raw": best["raw"], "count": len(ms), "latest": max(r["id"] for r in ms),
                    "exact": sum(r["kind"] == "exact" for r in ms), "similar": sum(r["kind"] == "similar" for r in ms),
                    "sources": [{"source": r["source"], "sender": r["sender"], "ts": r["ts"], "kind": r["kind"],
                                 "vendor": r["data"].get("vendor_name"), "phone": r["data"].get("vendor_phone")} for r in ms]})
    return sorted(out, key=lambda c: -c["latest"])

@app.get("/api/leads")
def api_leads(): return clusters()

@app.get("/api/export.csv")
def api_csv():
    cols = ["tech_stack", "start_date", "end_date", "duration_days", "college", "city", "state", "mode", "audience", "travel", "food",
            "accommodation", "logistics_note", "rate", "payment_date", "vendor_name", "vendor_company", "vendor_phone", "summary"]
    o = io.StringIO(); w = csv.writer(o); w.writerow(cols + ["times_seen", "groups"])
    for c in clusters():
        d = c["data"]; d["tech_stack"] = ", ".join(d.get("tech_stack") or [])
        w.writerow([d.get(k) for k in cols] + [c["count"], "; ".join(sorted({s["source"] for s in c["sources"]}))])
    return StreamingResponse(iter([o.getvalue()]), media_type="text/csv", headers={"Content-Disposition": "attachment; filename=trainlead.csv"})

app.mount("/", StaticFiles(directory=os.path.join(os.path.dirname(__file__), "..", "web"), html=True))
