#!/usr/bin/env python3
"""Apply the human's Kanvas decisions when they can only reach the agent through chat.

The Kanvas board UI (`kanvas.py serve`) is where the human normally approves
proposals and reviews work. In cloud sessions that page is unreachable, so the
human gives those decisions in the chat and the agent records them here.

ONLY run a command from this file when the human explicitly asked for that exact
action in the conversation. Every change is signed "human" with a note saying it
was relayed from chat by the agent.

Usage (from the repo root):
  python3 tools/kanvas_human.py approve UI-01 TD-01 ...
  python3 tools/kanvas_human.py reject  PR-02 ...          # delete proposals
  python3 tools/kanvas_human.py done    UI-01 [-m "note"]  # accept work in review
  python3 tools/kanvas_human.py sendback UI-01 -m "what needs to change"
  python3 tools/kanvas_human.py reopen  UI-01 [-m "why"]   # done -> todo
"""
import argparse
import importlib.util
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BOARD = os.environ.get("KANVAS_BOARD") or os.path.join(ROOT, "kanvas.json")
_spec = importlib.util.spec_from_file_location("kanvas", os.path.join(ROOT, ".agents", "skills", "kanvas", "kanvas.py"))
kv = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(kv)

RELAY = "(relayed from chat by the agent)"


def _note(task, text):
    task.setdefault("notes", []).append({"at": kv.now_iso(), "by": "human", "text": f"{text} {RELAY}".strip()})


def _move(board, ref, allowed, to, text):
    t = kv.find_task(board, ref)
    if t["state"] not in allowed:
        raise kv.KanvasError(f"{t['id']} is {t['state']}; this action needs it to be {' or '.join(allowed)}")
    t["state"] = to
    if to in ("todo", "proposed"):
        t.pop("started", None)
    _note(t, text)
    return t["id"]


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("action", choices=["approve", "reject", "done", "sendback", "reopen"])
    p.add_argument("ids", nargs="+")
    p.add_argument("-m", "--message", default="")
    a = p.parse_args()
    if a.action == "sendback" and not a.message.strip():
        p.error("sendback needs -m with what must change")

    def apply(board):
        out = []
        for ref in a.ids:
            if a.action == "approve":
                out.append(_move(board, ref, ["proposed"], "todo", a.message or "Approved."))
            elif a.action == "reject":
                t = kv.find_task(board, ref)
                if t["state"] != "proposed":
                    raise kv.KanvasError(f"{t['id']} is {t['state']}; only proposals can be rejected")
                board["tasks"] = [x for x in board["tasks"] if x is not t]
                for x in board["tasks"]:
                    x["deps"] = [d for d in x.get("deps", []) if d.upper() != t["id"].upper()]
                out.append(t["id"])
            elif a.action == "done":
                out.append(_move(board, ref, ["review", "doing"], "done", a.message or "Accepted."))
            elif a.action == "sendback":
                out.append(_move(board, ref, ["review"], "todo", f"Sent back: {a.message.strip()}"))
            elif a.action == "reopen":
                out.append(_move(board, ref, ["done"], "todo", a.message or "Reopened."))
        errors = kv.structural_errors(board)
        if errors:
            raise kv.KanvasError("; ".join(errors))
        return out

    try:
        ids = kv.mutate(BOARD, apply)
    except kv.KanvasError as e:
        sys.exit(f"Error: {e}")
    print(f"{a.action}: {', '.join(ids)}")


if __name__ == "__main__":
    main()
