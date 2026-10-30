// Check skill frontmatter, local references, AGENTS routing, and embedded files.
// Usage: node scripts/validate-skills.cjs
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');

const root = path.resolve(__dirname, '..');
const skillsRoot = path.join(root, '.agents', 'skills');
const catalogPath = path.join(root, 'crates', 'skills', 'src', 'catalog.rs');
const errors = [];

function filesBelow(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const fullPath = path.join(directory, entry.name);
    return entry.isDirectory() ? filesBelow(fullPath) : [fullPath];
  });
}

function markdownLinks(source) {
  const links = [];
  let inFence = false;
  for (const line of source.split(/\r?\n/)) {
    if (/^\s*```/.test(line)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    for (const match of line.matchAll(/!?\[[^\]]*\]\(([^)]+)\)/g)) {
      let target = match[1].trim().split(/\s+/, 1)[0];
      if (target.startsWith('<') && target.endsWith('>')) target = target.slice(1, -1);
      links.push(target);
    }
  }
  return links;
}

function catalogEntries(source) {
  const paths = new Set();
  for (const match of source.matchAll(/relative_path:\s*"([^"]+)"/g)) paths.add(match[1]);
  for (const match of source.matchAll(/skill_file!\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*\)/g)) {
    paths.add(`.agents/skills/${match[1]}/${match[2]}`);
  }
  return paths;
}

function resolveMarkdownTarget(fromFile, target) {
  if (!target || target.startsWith('#') || /^[a-z][a-z0-9+.-]*:/i.test(target)) return null;
  const localPath = target.split('#', 1)[0].split('?', 1)[0];
  if (!localPath) return null;
  const resolved = localPath.replace(/\\/g, '/').startsWith('.agents/')
    ? path.resolve(root, localPath)
    : path.resolve(path.dirname(fromFile), localPath);
  return resolved;
}

const sourceFiles = filesBelow(skillsRoot);
const skillPages = sourceFiles.filter(file => path.basename(file) === 'SKILL.md');
const skillByDirectory = new Map();
for (const file of skillPages) {
  const directory = path.basename(path.dirname(file));
  const contents = fs.readFileSync(file, 'utf8');
  const frontmatter = contents.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/);
  assert(frontmatter, `${path.relative(root, file)}: missing YAML frontmatter`);
  const name = frontmatter[1].match(/^name:\s*([^\r\n]+)$/m)?.[1]?.trim();
  const description = frontmatter[1].match(/^description:\s*(.+)$/m)?.[1]?.trim();
  assert(name, `${path.relative(root, file)}: missing name`);
  assert(description, `${path.relative(root, file)}: missing description`);
  assert.equal(name, directory, `${path.relative(root, file)}: name must match its directory`);
  assert(description.length <= 300, `${name}: keep the discovery description concise`);
  assert(!/\bSkill tool\b|\bsuperpowers:[a-z0-9-]+/i.test(contents),
    `${path.relative(root, file)}: avoid unavailable tool invocations; link local skills instead`);
  skillByDirectory.set(directory, file);
}

const catalogText = fs.readFileSync(catalogPath, 'utf8');
const embedded = catalogEntries(catalogText);
assert(embedded.size > 0, 'skill catalog has no embedded paths');
for (const relative of embedded) {
  assert(fs.existsSync(path.join(root, relative)), `catalog source is missing: ${relative}`);
}
for (const file of skillPages) {
  const relative = path.relative(root, file).replace(/\\/g, '/');
  assert(embedded.has(relative), `SKILL.md is not embedded in catalog: ${relative}`);
}

const linkedPages = new Map();
const markdownFiles = sourceFiles.filter(file => file.endsWith('.md'));
const agentsFile = path.join(root, 'AGENTS.md');
markdownFiles.push(agentsFile, path.join(root, 'docs', 'SKILLS-STRUCTURE-REVIEW.md'));
for (const file of markdownFiles) {
  const source = fs.readFileSync(file, 'utf8');
  const pageEdges = [];
  for (const target of markdownLinks(source)) {
    const resolved = resolveMarkdownTarget(file, target);
    if (!resolved) continue;
    assert(fs.existsSync(resolved), `${path.relative(root, file)}: broken link ${target}`);
    const relative = path.relative(root, resolved).replace(/\\/g, '/');
    if (relative.startsWith('.agents/skills/')) {
      assert(embedded.has(relative), `${path.relative(root, file)}: linked file is not bundled: ${relative}`);
      if (path.basename(relative) === 'SKILL.md') pageEdges.push(path.basename(path.dirname(relative)));
    }
  }
  if (path.basename(file) === 'SKILL.md') linkedPages.set(path.basename(path.dirname(file)), pageEdges);
}

const agentsText = fs.readFileSync(agentsFile, 'utf8');
const routed = new Set([...agentsText.matchAll(/\.agents\/skills\/([^`\s]+)\/SKILL\.md/g)]
  .map(match => match[1]));
for (const required of [
  'harness-setup', 'harness-read', 'harness-edit', 'harness-browser',
  'harness-sql', 'harness-xlsx', 'harness-svg', 'harness-workflow',
]) {
  assert(routed.has(required), `AGENTS.md is missing task routing for ${required}`);
}

const visiting = new Set();
const visited = new Set();
function visit(name) {
  if (visiting.has(name)) throw new Error(`skill link cycle includes ${name}`);
  if (visited.has(name)) return;
  visiting.add(name);
  for (const next of linkedPages.get(name) || []) if (skillByDirectory.has(next)) visit(next);
  visiting.delete(name);
  visited.add(name);
}
for (const name of skillByDirectory.keys()) visit(name);

const charCounts = skillPages.map(file => ({
  skill: path.basename(path.dirname(file)),
  chars: fs.readFileSync(file, 'utf8').length,
})).sort((a, b) => b.chars - a.chars);
console.log(JSON.stringify({
  skills: skillPages.length,
  entrypointChars: charCounts.reduce((sum, item) => sum + item.chars, 0),
  embeddedFiles: embedded.size,
  localMarkdownLinksResolved: true,
  frontmatterValid: true,
  agentsRoutingValid: true,
  skillLinkCycles: 0,
  charCounts,
}, null, 2));
