# Agent instructions

Plan and track work on the Kanvas board; follow .agents/skills/kanvas/SKILL.md.

## When the human can only reach you through chat

In cloud sessions the human cannot open the Kanvas web board (`kanvas.py serve` runs inside the container). Then the chat replaces the board for the human's own decisions:

- Show the board to the human in chat (`kanvas.py status`, `show ID`) instead of pointing them to the web page.
- When the human explicitly tells you in chat to approve, reject, accept (mark done), send back or reopen tasks, record exactly that with `python3 tools/kanvas_human.py <approve|reject|done|sendback|reopen> ID... [-m TEXT]`. It signs the change as the human, notes that it was relayed from chat, and validates the board.
- Never use it on your own initiative or stretch what was said: "start TD-01" is not approval of other tasks, and finishing your own work still goes to review until the human accepts it in chat.
- After changing the board, commit and push `kanvas.json` so the human's decisions are not lost when the container is reclaimed.
