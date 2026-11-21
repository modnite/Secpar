# Secpar blog

secpar.win/blog is built with [Hugo](https://gohugo.io) (Apache-2.0) and edited with
[Decap CMS](https://decapcms.org) (MIT). Published pages are static HTML with no
JavaScript; the site worker in `cloud/site` serves them and adds approved comments.

## Writing posts

Open https://secpar.win/blog/admin/ and sign in with a GitHub account that can push to
this repository. Publish commits the post to `main` under `content/posts`, and the
"Deploy Secpar website" workflow puts it live about two minutes later. Ticking Draft
keeps a saved post off the blog. Images uploaded in the editor go to `static/images`.

Posts can also be written by hand as `content/posts/<name>.md` with `title` and `date`
front matter, or as `YYYY-MM-DD-name.md`, which takes its date from the file name.

## Comments

Readers comment anonymously; nothing shows until approved at
https://secpar.win/blog/admin/comments with the moderator key.
