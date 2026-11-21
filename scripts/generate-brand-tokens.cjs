#!/usr/bin/env node
// CCS/1 IO:WN-167 assets/brand-tokens.json -> the generated block of apps/windows/frontend/tokens.css and
// apps/android/.../ui/BrandColors.kt. --check: exit 1 when an output differs from what the source gives (CI).
'use strict';
const fs = require('fs');
const path = require('path');

const root = path.resolve(__dirname, '..');
const source = path.join(root, 'assets', 'brand-tokens.json');
const css = path.join(root, 'apps', 'windows', 'frontend', 'tokens.css');
const kotlin = path.join(root, 'apps', 'android', 'app', 'src', 'main', 'java', 'io', 'github', 'prejas3', 'windconnect', 'ui', 'BrandColors.kt');
const BEGIN = '/* BRAND-TOKENS:BEGIN generated from assets/brand-tokens.json by scripts/generate-brand-tokens.cjs; do not edit by hand */';
const END = '/* BRAND-TOKENS:END */';
// The roles of material3's lightColorScheme()/darkColorScheme(), in their order; each must be set.
const ROLES = ['primary', 'onPrimary', 'primaryContainer', 'onPrimaryContainer', 'inversePrimary', 'secondary', 'onSecondary',
  'secondaryContainer', 'onSecondaryContainer', 'tertiary', 'onTertiary', 'tertiaryContainer', 'onTertiaryContainer', 'background',
  'onBackground', 'surface', 'onSurface', 'surfaceVariant', 'onSurfaceVariant', 'surfaceTint', 'inverseSurface', 'inverseOnSurface',
  'error', 'onError', 'errorContainer', 'onErrorContainer', 'outline', 'outlineVariant', 'scrim', 'surfaceBright', 'surfaceContainer',
  'surfaceContainerHigh', 'surfaceContainerHighest', 'surfaceContainerLow', 'surfaceContainerLowest', 'surfaceDim'];

const tokens = JSON.parse(fs.readFileSync(source, 'utf8'));
const problems = [];
for (const theme of ['light', 'dark']) {
  const roles = tokens[theme].roles;
  for (const role of ROLES) if (!/^#[0-9A-F]{6}$/.test(roles[role] || '')) problems.push(`${theme}.${role}: missing or not #RRGGBB`);
  for (const role of Object.keys(roles)) if (!ROLES.includes(role)) problems.push(`${theme}.${role}: not a Material 3 role`);
}
if (problems.length) { console.error(problems.join('\n')); process.exit(1); }

function value(theme, ref) {
  const found = ref.startsWith('extra.') ? tokens[theme].extra[ref.slice(6)] : tokens[theme].roles[ref];
  if (found === undefined) throw new Error(`windows mapping ${ref}: no such token in ${theme}`);
  return /^#[0-9A-F]+$/i.test(found) ? found.toLowerCase() : found.replace(/#[0-9A-F]+/gi, (h) => h.toLowerCase());
}
function cssBlock() {
  const rule = (selector, theme, scheme) => {
    const decls = Object.entries(tokens.windows).map(([name, ref]) => `${name}: ${value(theme, ref)};`);
    const lines = [];
    for (let i = 0; i < decls.length; i += 6) lines.push('  ' + decls.slice(i, i + 6).join(' '));
    return `${selector} {\n${lines.join('\n')}\n  color-scheme: ${scheme};\n}`;
  };
  return [BEGIN, rule(':root', 'light', 'light'), rule(':root[data-theme="dark"]', 'dark', 'dark'), END].join('\n');
}
function kotlinFile() {
  const scheme = (name, fn, theme) => {
    const args = ROLES.map((role) => `    ${role} = Color(0xFF${tokens[theme].roles[role].slice(1)}),`);
    return `val ${name} = ${fn}(\n${args.join('\n')}\n)`;
  };
  return [
    '// GENERATED from assets/brand-tokens.json by scripts/generate-brand-tokens.cjs; do not edit by hand.',
    'package io.github.prejas3.windconnect.ui',
    '',
    'import androidx.compose.material3.darkColorScheme',
    'import androidx.compose.material3.lightColorScheme',
    'import androidx.compose.ui.graphics.Color',
    '',
    '// CCS/1 RULE:WN-167 every role is the brand\'s, none the baseline M3 purple; Android 10/11 and Material You off use these.',
    scheme('BrandLight', 'lightColorScheme', 'light'),
    scheme('BrandDark', 'darkColorScheme', 'dark'),
    '',
  ].join('\n');
}

const norm = (text) => text.replace(/\r\n/g, '\n');
const current = fs.readFileSync(css, 'utf8');
const eol = current.includes('\r\n') ? '\r\n' : '\n';
const plain = norm(current);
const start = plain.indexOf(BEGIN);
const end = plain.indexOf(END);
if (start < 0 || end < start) { console.error(`${path.relative(root, css)}: no ${BEGIN} … ${END} block`); process.exit(1); }
const wantedCss = plain.slice(0, start) + cssBlock() + plain.slice(end + END.length);
const wantedKotlin = kotlinFile();
const outputs = [[css, wantedCss, eol], [kotlin, wantedKotlin, fs.existsSync(kotlin) && fs.readFileSync(kotlin, 'utf8').includes('\r\n') ? '\r\n' : '\n']];

if (process.argv.includes('--check')) {
  const stale = outputs.filter(([file, wanted]) => !fs.existsSync(file) || norm(fs.readFileSync(file, 'utf8')) !== wanted);
  for (const [file] of stale) console.error(`${path.relative(root, file)} is out of date: run node scripts/generate-brand-tokens.cjs`);
  if (stale.length) process.exit(1);
  console.log('brand tokens: outputs match assets/brand-tokens.json');
} else {
  for (const [file, wanted, newline] of outputs) fs.writeFileSync(file, wanted.replace(/\n/g, newline));
  console.log('brand tokens: wrote ' + outputs.map(([file]) => path.relative(root, file)).join(', '));
}
