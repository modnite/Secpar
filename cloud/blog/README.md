# Secpar blog

blog.secpar.win is built with [Hugo](https://gohugo.io) (Apache-2.0). The output is
static HTML with no JavaScript. The site worker in `cloud/site` serves it, using the
main site's stylesheet, font and logo.

## Add a post

Create `content/posts/YYYY-MM-DD-short-title.md`. The date in the file name is the
post's date and the rest becomes its address, so `2026-11-20-secpar-4-1.md` is
published at `https://blog.secpar.win/secpar-4-1/` and dated 20 November 2026.

```markdown
---
title: Secpar 4.1
---

The first paragraph is the summary shown in the post list and the RSS feed.

## A heading

Markdown as usual: lists, links, `code`, quotes and images.
```

Optional front matter: `draft: true` keeps a post out of the build; `date:` overrides
the file name's date; `lastmod:` adds an "Updated" line. Put images next to the post
by making a folder instead of a file: `content/posts/2026-11-20-secpar-4-1/index.md`
with `screen.png` beside it, linked as `![What it shows](screen.png)`.

Push to `main` and the "Deploy Secpar website" workflow builds and publishes it.

## Preview

With Hugo 0.170.1 or later: `hugo server --source cloud/blog`. The page styles come
from the main site, so a local preview is unstyled unless that site is served too.
