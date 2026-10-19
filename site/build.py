#!/usr/bin/env python3
"""Builds the retake website into a static directory for GitHub Pages.

    python site/build.py _site

Standard library only. Each file in pages/ starts with a front-matter block:

    ---
    title: Page title (50-65 characters is ideal)
    description: Meta description (120-160 characters is ideal)
    path: guides/example/        # URL path below the site root; "" for the home page
    type: article                # website | article
    breadcrumb: Guides=guides/; Example=guides/example/
    published: 2026-10-19
    noindex: true                # optional
    ---

The build fails on missing front matter, broken internal links, or links to
pages that don't exist, so a bad deploy never goes out.
"""

import base64
import datetime as dt
import hashlib
import html
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path
from string import Template

SITE = "https://zecrari.github.io"
BASE = "/retake/"
ROOT = Path(__file__).resolve().parent

# Applied before first paint so there's no flash of the wrong theme. Its hash goes in the CSP.
THEME_SCRIPT = (
    'try{var t=localStorage.getItem("retake-theme");'
    'if(t==="light"||t==="dark")document.documentElement.setAttribute("data-theme",t)}catch(e){}'
)

NAV = [("Docs", "docs/"), ("Guides", "guides/")]


def fail(message):
    print(f"build error: {message}", file=sys.stderr)
    sys.exit(1)


def parse_page(path):
    text = path.read_text(encoding="utf-8")
    match = re.match(r"---\n(.*?)\n---\n", text, re.S)
    if not match:
        fail(f"{path}: missing front matter")
    meta = {}
    for line in match.group(1).splitlines():
        if line.strip():
            key, _, value = line.partition(":")
            meta[key.strip()] = value.strip()
    for key in ("title", "description", "path"):
        if key not in meta:
            fail(f"{path}: front matter needs `{key}`")
    meta["content"] = text[match.end():]
    meta["source"] = path
    return meta


def minify_css(css):
    css = re.sub(r"/\*.*?\*/", "", css, flags=re.S)
    css = re.sub(r"\s+", " ", css)
    css = re.sub(r"\s*([{}:;,>])\s*", r"\1", css)
    return css.replace(";}", "}").strip()


def fingerprint(out, rel, content):
    digest = hashlib.sha256(content.encode()).hexdigest()[:10]
    stem, ext = rel.rsplit(".", 1)
    hashed = f"{stem}.{digest}.{ext}"
    (out / hashed).write_text(content, encoding="utf-8")
    (out / rel).unlink()
    return hashed


def last_modified(path):
    try:
        out = subprocess.run(
            ["git", "log", "-1", "--format=%cs", "--", str(path)],
            capture_output=True, text=True, check=True, cwd=ROOT,
        ).stdout.strip()
        if out:
            return out
    except (OSError, subprocess.CalledProcessError):
        pass
    return dt.date.today().isoformat()


def breadcrumb_jsonld(meta):
    items = [("retake", "")]
    for part in filter(None, (p.strip() for p in meta.get("breadcrumb", "").split(";"))):
        name, _, rel = part.partition("=")
        items.append((name.strip(), rel.strip()))
    if len(items) < 2:
        return ""
    data = {
        "@context": "https://schema.org",
        "@type": "BreadcrumbList",
        "itemListElement": [
            {"@type": "ListItem", "position": i + 1, "name": name, "item": SITE + BASE + rel}
            for i, (name, rel) in enumerate(items)
        ],
    }
    return '<script type="application/ld+json">' + json.dumps(data, separators=(",", ":")) + "</script>"


def article_jsonld(meta, url):
    if meta.get("type") != "article":
        return ""
    data = {
        "@context": "https://schema.org",
        "@type": "TechArticle",
        "headline": meta["title"],
        "description": meta["description"],
        "url": url,
        "image": SITE + BASE + "og.png",
        "inLanguage": "en",
        "author": {"@type": "Organization", "name": "Zecrari", "url": "https://github.com/Zecrari"},
        "publisher": {"@type": "Organization", "name": "Zecrari", "url": "https://github.com/Zecrari"},
        "about": {"@type": "SoftwareApplication", "name": "retake", "applicationCategory": "DeveloperApplication"},
    }
    if meta.get("published"):
        data["datePublished"] = meta["published"]
        data["dateModified"] = meta["modified"]
    return '<script type="application/ld+json">' + json.dumps(data, separators=(",", ":")) + "</script>"


def breadcrumb_html(meta):
    parts = [p.strip() for p in meta.get("breadcrumb", "").split(";") if p.strip()]
    if not parts:
        return ""
    links = [f'<a href="{BASE}">Home</a>']
    for i, part in enumerate(parts):
        name, _, rel = part.partition("=")
        if i == len(parts) - 1:
            links.append(f'<span aria-current="page">{html.escape(name.strip())}</span>')
        else:
            links.append(f'<a href="{BASE}{rel.strip()}">{html.escape(name.strip())}</a>')
    return '<nav class="wrap breadcrumbs" aria-label="Breadcrumb">' + '<span aria-hidden="true">/</span>'.join(links) + "</nav>"


def nav_html(path):
    links = []
    for name, rel in NAV:
        current = ' aria-current="page"' if path.startswith(rel) else ""
        links.append(f'<a href="{BASE}{rel}"{current}>{name}</a>')
    return "\n        ".join(links)


def main():
    if len(sys.argv) != 2:
        fail("usage: build.py OUTPUT_DIR")
    out = Path(sys.argv[1]).resolve()
    if out.exists():
        shutil.rmtree(out)
    shutil.copytree(ROOT / "static", out)

    css = fingerprint(out, "assets/site.css", minify_css((out / "assets/site.css").read_text(encoding="utf-8")))
    js = fingerprint(out, "assets/site.js", (out / "assets/site.js").read_text(encoding="utf-8"))
    theme_hash = base64.b64encode(hashlib.sha256(THEME_SCRIPT.encode()).digest()).decode()
    csp = (
        "default-src 'self'; "
        f"script-src 'self' 'sha256-{theme_hash}'; "
        "style-src 'self' 'unsafe-inline'; "
        "img-src 'self' data:; font-src 'self'; connect-src 'self'; manifest-src 'self'; "
        "object-src 'none'; base-uri 'self'; form-action 'none'"
    )

    layout = Template((ROOT / "layout.html").read_text(encoding="utf-8"))
    pages = [parse_page(p) for p in sorted((ROOT / "pages").rglob("*.html"))]
    built = set()
    sitemap = []

    for meta in pages:
        rel = meta["path"]
        is_404 = meta["source"].name == "404.html"
        target = out / "404.html" if is_404 else out / rel / "index.html"
        target.parent.mkdir(parents=True, exist_ok=True)
        url = SITE + BASE + rel
        meta["modified"] = last_modified(meta["source"])
        noindex = meta.get("noindex") == "true"

        if not 30 <= len(meta["title"]) <= 70 and not is_404:
            print(f"warning: {rel or '/'} title is {len(meta['title'])} characters", file=sys.stderr)
        if not 110 <= len(meta["description"]) <= 165 and not is_404:
            print(f"warning: {rel or '/'} description is {len(meta['description'])} characters", file=sys.stderr)

        page = layout.substitute(
            title=html.escape(meta["title"]),
            og_title=html.escape(meta.get("og_title", meta["title"])),
            description=html.escape(meta["description"]),
            canonical=url,
            og_type=meta.get("type", "website"),
            robots="noindex, follow" if noindex else "index, follow, max-image-preview:large, max-snippet:-1",
            base=BASE,
            site=SITE,
            css=BASE + css,
            js=BASE + js,
            csp=csp,
            theme_script=THEME_SCRIPT,
            jsonld=breadcrumb_jsonld(meta) + article_jsonld(meta, url),
            nav=nav_html(rel),
            breadcrumbs=breadcrumb_html(meta),
            content=meta["content"].replace("{{base}}", BASE),
            year=dt.date.today().year,
        )
        target.write_text(page, encoding="utf-8")
        built.add(rel)
        if not noindex and not is_404:
            sitemap.append((url, meta["modified"], "1.0" if rel == "" else "0.8"))

    entries = "\n".join(
        f"  <url><loc>{loc}</loc><lastmod>{mod}</lastmod><priority>{prio}</priority></url>"
        for loc, mod, prio in sorted(sitemap)
    )
    (out / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n' + entries + "\n</urlset>\n",
        encoding="utf-8",
    )

    # Every internal link must point at a page or file we built.
    for file in out.rglob("*.html"):
        text = file.read_text(encoding="utf-8")
        for href in re.findall(r'(?:href|src)="(/retake/[^"#?]*)', text):
            rel = href[len(BASE):]
            if rel.endswith("/") or rel == "":
                if rel not in built:
                    fail(f"{file.relative_to(out)}: broken link {href}")
            elif not (out / rel).exists():
                fail(f"{file.relative_to(out)}: missing file {href}")
        for anchor in re.findall(r'href="#([^"]+)"', text):
            if f'id="{anchor}"' not in text:
                fail(f"{file.relative_to(out)}: broken anchor #{anchor}")

    print(f"built {len(pages)} pages into {out}")


if __name__ == "__main__":
    main()
