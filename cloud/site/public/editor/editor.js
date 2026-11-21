// Decap CMS setup for secpar.win/blog/admin. The preview uses the blog's own styles and
// the same structure as a published post page.
window.CMS.registerPreviewStyle('/style.css');
window.CMS.registerPreviewStyle('/assets/blog.css');
const h = window.h;
const PostPreview = window.createClass({
  render() {
    const data = this.props.entry.get('data');
    const date = data.get('date');
    const shown = date ? new Date(`${date}`.slice(0, 10) + 'T00:00:00Z').toLocaleDateString('en-GB', { day: 'numeric', month: 'long', year: 'numeric', timeZone: 'UTC' }) : '';
    return h('main', { className: 'wrap blog' },
      h('article', { className: 'post' },
        shown ? h('time', {}, shown) : null,
        h('h1', {}, data.get('title') || ''),
        this.props.widgetFor('body')));
  },
});
window.CMS.registerPreviewTemplate('posts', PostPreview);
