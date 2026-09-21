import { cp, mkdir, rm } from 'node:fs/promises';
const output = new URL('./dist/', import.meta.url);
await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });
for (const item of ['index.html', 'style.css', 'main.js', 'assets']) {
  await cp(new URL(item, import.meta.url), new URL(item, output), { recursive: true });
}
// Public builds omit third-party screenshots and the internal reference board.
const { readFile, writeFile } = await import('node:fs/promises');
const index = new URL('index.html', output);
await writeFile(index, (await readFile(index, 'utf8')).replace('<a href="references.html">디자인 레퍼런스 ↗</a>', '<a href="#faq">자주 묻는 질문 ↗</a>'));
console.log('Built static homepage in site/dist (internal reference board excluded).');
