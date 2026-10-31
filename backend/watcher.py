# Desktop: watch the folder where you save "Export chat" files and ingest each new .txt automatically.
import os, sys, time, requests
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler
API = os.getenv("API", "http://127.0.0.1:8000")
class H(FileSystemEventHandler):
    def on_created(self, e):
        if e.src_path.lower().endswith(".txt"):
            time.sleep(1)
            with open(e.src_path, "rb") as f:
                print(e.src_path, requests.post(API + "/api/ingest/export", files={"file": (os.path.basename(e.src_path), f)}).json())
o = Observer(); o.schedule(H(), sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/Downloads")); o.start()
try:
    while True: time.sleep(1)
except KeyboardInterrupt: o.stop()
