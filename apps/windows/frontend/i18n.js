// WN-63: every UI text in Polish and English. The language follows Windows (WebView2 reports the display language
// as navigator.language); anything but Polish gets English. Pages mark static texts with data-i18n (text) and
// data-i18n-<attribute> (aria-label, placeholder, title, alt); scripts use t() and plural().
import pl from './locales/pl.js';
import en from './locales/en.js';

const preferred = (navigator.languages && navigator.languages[0]) || navigator.language || 'en';
export const lang = preferred.toLowerCase().startsWith('pl') ? 'pl' : 'en';
/** The BCP 47 tag for Intl formatting (dates, times, numbers). */
export const locale = lang === 'pl' ? 'pl-PL' : (preferred.toLowerCase().startsWith('en') ? preferred : 'en-US');
const table = lang === 'pl' ? pl : en;

function fill(text, params) {
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (match, name) => (name in params ? String(params[name]) : match));
}

/** The text for key, with {name} placeholders filled from params. A missing key falls back to English, then the key. */
export function t(key, params) {
  return fill(table[key] ?? en[key] ?? key, params);
}

const rules = new Intl.PluralRules(locale);
/** Key's plural form for count (key.one, key.few, key.many, key.other); {count} is filled in. */
export function plural(key, count, params) {
  const form = rules.select(count);
  const text = table[`${key}.${form}`] ?? table[`${key}.other`] ?? en[`${key}.${form}`] ?? en[`${key}.other`] ?? key;
  return fill(text, { count: formatNumber(count), ...params });
}

export const formatNumber = (value, options) => new Intl.NumberFormat(locale, options).format(value);
export const formatTime = (value, options = { hour: '2-digit', minute: '2-digit' }) => new Date(value).toLocaleTimeString(locale, options);
export const formatDate = (value, options) => new Date(value).toLocaleString(locale, options);

const ATTRIBUTES = ['aria-label', 'placeholder', 'title', 'alt', 'aria-description', 'value'];

function setText(element, text) {
  if (!element.firstElementChild) { element.textContent = text; return; }
  // Text next to an icon: keep the icon, replace (or add) the element's own text node.
  const own = [...element.childNodes].find(node => node.nodeType === Node.TEXT_NODE && node.nodeValue.trim());
  if (own) { own.nodeValue = text; return; }
  const node = document.createTextNode(text);
  if (element.dataset.i18nPos === 'start') element.prepend(node); else element.append(node);
}

/** Fills root's marked texts and attributes in the UI language. */
export function applyI18n(root = document) {
  if (root.matches?.('[data-i18n]')) setText(root, t(root.dataset.i18n));
  for (const element of root.querySelectorAll('[data-i18n]')) setText(element, t(element.dataset.i18n));
  for (const name of ATTRIBUTES) {
    for (const element of root.querySelectorAll(`[data-i18n-${name}]`)) element.setAttribute(name, t(element.getAttribute(`data-i18n-${name}`)));
  }
}

document.documentElement.lang = lang;
applyI18n();
