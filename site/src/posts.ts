// The blog posts (src/pages/blog/*.md), newest first, for the blog index and
// the sitemap. URLs end in "/", like every other page on the site.
import { postDate } from "./dates";

type Frontmatter = { title: string; description: string; date: string | Date };
type Page = { url: string; frontmatter: Frontmatter };

const pages = Object.values(import.meta.glob<Page>("./pages/blog/*.md", { eager: true }));

export const posts = pages
  .map(({ url, frontmatter }) => ({
    url: url.endsWith("/") ? url : `${url}/`,
    title: frontmatter.title,
    description: frontmatter.description,
    ...postDate(frontmatter.date),
  }))
  .sort((a, b) => b.iso.localeCompare(a.iso));
