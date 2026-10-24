"""An animated SVG of cockpit's notifications sliding in, each with where its click leads and the buttons it carries."""
import html, os

HERE = os.path.dirname(os.path.abspath(__file__))
ITEMS = [
    ("📥 Inbox", "👀 Review requested: web#1840 Quota copy", "the PR", ["Review"]),
    ("🔁 CircleCI", "🔴 CI failed: web me/eng-413-quota-ui", "the PR's checks", ["Why?", "Rerun"]),
    ("✓ quota-ui", "Checked on phone, tablet and desktop", "back to that agent's space", []),
    ("📥 Inbox", "🎫 Assigned: ENG-420 Quota emails", "the ticket", ["Plan"]),
    ("📅 Meeting", "Quotas design review in 5 min", "its brief", ["Join"]),
    ("⏸ invoice-csv", "Claude is waiting for you", "the agent that needs you", []),
]
W, TOP, STEP, BW, BH, BX = 980, 28, 88, 480, 74, 470
H = TOP * 2 + STEP * len(ITEMS) - (STEP - BH) + 26
LOOP, SLOT = 9.0, 0.7
SANS = "-apple-system,BlinkMacSystemFont,'SF Pro Text','Segoe UI',Helvetica,Arial,sans-serif"


def logo(x, y, size):
    """The cockpit logo, drawn from the same shapes as docs/logo.svg."""
    k = size / 512
    return f'''<g transform="translate({x} {y}) scale({k})">
  <rect x="8" y="8" width="496" height="496" rx="112" fill="#101010" stroke="#2C2C30" stroke-width="8"/>
  <path d="M 149.9 376.1 A 150 150 0 1 1 362.1 376.1" fill="none" stroke="#2C2C30" stroke-width="34" stroke-linecap="round"/>
  <path d="M 149.9 376.1 A 150 150 0 1 1 378.9 183.9" fill="none" stroke="#FFC799" stroke-width="34" stroke-linecap="round"/>
  <line x1="256" y1="270" x2="346.1" y2="206.9" stroke="#F5F5F5" stroke-width="16" stroke-linecap="round"/>
  <circle cx="256" cy="270" r="24" fill="#FFC799"/>
  <circle cx="210" cy="420" r="13" fill="#99FFE4"/><circle cx="256" cy="420" r="13" fill="#FFC799"/><circle cx="302" cy="420" r="13" fill="#505050"/>
</g>'''


def buttons_svg(buttons, y):
    """The notification's buttons, stacked at its right edge as macOS draws them."""
    out, bw, bh = [], 64, 24
    top = y + (BH - len(buttons) * bh - (len(buttons) - 1) * 6) / 2
    for n, label in enumerate(buttons):
        by = top + n * (bh + 6)
        out.append(f'<rect x="{BX + BW - 16 - bw}" y="{by}" width="{bw}" height="{bh}" rx="7" fill="#3A3A44" stroke="#4A4A54"/>'
                   f'<text x="{BX + BW - 16 - bw / 2}" y="{by + 16}" text-anchor="middle" fill="#F2F2F2" font-size="12">{html.escape(label)}</text>')
    return "".join(out)


style, groups = "", []
for i, (title, body, click, buttons) in enumerate(ITEMS):
    y = TOP + i * STEP
    a, b = 100 * (0.4 + i * SLOT) / LOOP, 100 * (0.4 + i * SLOT + 0.45) / LOOP
    style += (f"@keyframes n{i}{{0%,{a:.1f}%{{transform:translateX(460px);opacity:0}}{b:.1f}%,88%{{transform:translateX(0);opacity:1}}"
              f"96%,100%{{transform:translateX(0);opacity:0}}}}.n{i}{{animation:n{i} {LOOP}s cubic-bezier(.2,.8,.2,1) infinite}}"
              f"@keyframes c{i}{{0%,{b:.1f}%{{opacity:0}}{b + 4:.1f}%,88%{{opacity:1}}96%,100%{{opacity:0}}}}.c{i}{{animation:c{i} {LOOP}s linear infinite}}")
    groups.append(f'''<g class="c{i}" opacity="0">
  <text x="{BX - 22}" y="{y + BH / 2 + 5}" text-anchor="end" fill="#FFC799" font-size="14">click → {html.escape(click)}</text>
</g>
<g class="n{i}">
  <rect x="{BX}" y="{y}" width="{BW}" height="{BH}" rx="18" fill="#2A2A30" fill-opacity="0.96" stroke="#3A3A42"/>
  {logo(BX + 14, y + 17, 40)}
  <text x="{BX + 66}" y="{y + 24}" fill="#9A9AA2" font-size="11" letter-spacing="0.6">COCKPIT</text>
  <text x="{BX + BW - 16}" y="{y + 24}" text-anchor="end" fill="#9A9AA2" font-size="11">now</text>
  <text x="{BX + 66}" y="{y + 43}" fill="#F2F2F2" font-size="14" font-weight="600">{html.escape(title)}</text>
  <text x="{BX + 66}" y="{y + 61}" fill="#C8C8CE" font-size="13">{html.escape(body)}</text>
  {buttons_svg(buttons, y)}
</g>''')

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">
<defs><linearGradient id="desk" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#1B1B24"/><stop offset="1" stop-color="#0F0F12"/></linearGradient></defs>
<style>text{{font-family:{SANS}}}{style}@media (prefers-reduced-motion:reduce){{g{{animation:none!important;opacity:1!important}}}}</style>
<rect width="100%" height="100%" rx="12" fill="url(#desk)" stroke="#2C2C30"/>
<text x="28" y="{H - 22}" fill="#707070" font-size="12">macOS notifications from cockpit: a click leads somewhere useful, a button acts</text>
{"".join(groups)}
</svg>'''
open(os.path.join(HERE, "..", "notifications.svg"), "w").write(svg)
