// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/sources/browser-cookie-source-partition.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

/**
 * Which site partition a source cookie belongs to. An unreadable one is left out of the import: writing it
 * unpartitioned would put it in the wrong jar.
 * @typedef {{status: "unpartitioned"} | {status: "partitioned", partitionKey: {topLevelSite: string, hasCrossSiteAncestor: boolean}} | {status: "unreadable", reason: string}} SourcePartition
 */

const CHROMIUM_PARTITION_SITE_COLUMN = "top_frame_site_key";
const CHROMIUM_CROSS_SITE_ANCESTOR_COLUMN = "has_cross_site_ancestor";
const FIREFOX_PARTITIONED_ATTRIBUTE_COLUMN = "isPartitionedAttributeSet";

/** @type {SourcePartition} */
export const UNPARTITIONED = Object.freeze({ status: "unpartitioned" });

function readSqliteFlag(raw) {
  if (typeof raw === "boolean") return raw;
  if (typeof raw === "bigint") return raw === 0n ? false : raw === 1n ? true : null;
  if (typeof raw === "number") return raw === 0 ? false : raw === 1 ? true : null;
  return null;
}

/** A partition's top-level site as a bare http(s) origin, or null when it is anything else. */
export function normalizeCookiePartitionSite(raw) {
  try {
    const site = new URL(raw);
    if (site.protocol !== "http:" && site.protocol !== "https:") return null;
    if (!site.hostname || site.username || site.password || site.port || site.pathname !== "/" || site.search || site.hash) return null;
    return site.origin;
  } catch {
    return null;
  }
}

/**
 * The partition of a Chromium `cookies` row, which needs both the site and the cross-site-ancestor bit.
 * @param {Record<string, unknown>} sourceRow
 * @param {ReadonlySet<string>} sourceColumns
 * @returns {SourcePartition}
 */
export function readChromiumRowPartition(sourceRow, sourceColumns) {
  // A schema without this column is older than partitioned cookies.
  if (!sourceColumns.has(CHROMIUM_PARTITION_SITE_COLUMN)) return UNPARTITIONED;
  const rawSite = sourceRow[CHROMIUM_PARTITION_SITE_COLUMN];
  if (rawSite === "") return UNPARTITIONED;
  if (typeof rawSite !== "string") return { status: "unreadable", reason: "partition site column was not text" };
  const topLevelSite = normalizeCookiePartitionSite(rawSite);
  if (!topLevelSite) return { status: "unreadable", reason: "partition site column was not a valid schemeful site" };
  if (!sourceColumns.has(CHROMIUM_CROSS_SITE_ANCESTOR_COLUMN)) {
    return { status: "unreadable", reason: "source schema has no cross-site-ancestor column for a partitioned cookie" };
  }
  const hasCrossSiteAncestor = readSqliteFlag(sourceRow[CHROMIUM_CROSS_SITE_ANCESTOR_COLUMN]);
  if (hasCrossSiteAncestor === null) return { status: "unreadable", reason: "cross-site-ancestor column was not an integer flag" };
  return { status: "partitioned", partitionKey: { topLevelSite, hasCrossSiteAncestor } };
}

/**
 * The partition of a Firefox `moz_cookies` row: its Partitioned attribute has no cross-site-ancestor bit to read.
 * @param {Record<string, unknown>} sourceRow
 * @param {ReadonlySet<string>} sourceColumns
 * @returns {SourcePartition}
 */
export function readFirefoxRowPartition(sourceRow, sourceColumns) {
  if (!sourceColumns.has(FIREFOX_PARTITIONED_ATTRIBUTE_COLUMN)) return UNPARTITIONED;
  const partitionedAttribute = readSqliteFlag(sourceRow[FIREFOX_PARTITIONED_ATTRIBUTE_COLUMN]);
  if (partitionedAttribute === false) return UNPARTITIONED;
  if (partitionedAttribute === null) return { status: "unreadable", reason: "partitioned-attribute column was not an integer flag" };
  return { status: "unreadable", reason: "Firefox partitioned-attribute cookie has no cross-site-ancestor bit to read" };
}

/**
 * The partition of a cookie-file entry: none when the fields are absent, unreadable when they are incomplete.
 * @param {unknown} raw The entry's `partitionKey`.
 * @param {unknown} [partitionKeyOpaque]
 * @returns {SourcePartition}
 */
export function readJsonCookiePartition(raw, partitionKeyOpaque = undefined) {
  if (partitionKeyOpaque === true) return { status: "unreadable", reason: "partition key was opaque" };
  if (partitionKeyOpaque !== undefined && typeof partitionKeyOpaque !== "boolean") {
    return { status: "unreadable", reason: "partitionKeyOpaque was not a boolean" };
  }
  if (raw === undefined) return UNPARTITIONED;
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    return { status: "unreadable", reason: "partitionKey was not an object with both fields" };
  }
  const { topLevelSite, hasCrossSiteAncestor } = raw;
  if (typeof topLevelSite !== "string" || topLevelSite.length === 0) {
    return { status: "unreadable", reason: "partitionKey.topLevelSite was missing or not text" };
  }
  const normalizedTopLevelSite = normalizeCookiePartitionSite(topLevelSite);
  if (!normalizedTopLevelSite) return { status: "unreadable", reason: "partitionKey.topLevelSite was not a valid schemeful site" };
  if (typeof hasCrossSiteAncestor !== "boolean") {
    return { status: "unreadable", reason: "partitionKey.hasCrossSiteAncestor was missing or not a boolean" };
  }
  return { status: "partitioned", partitionKey: { topLevelSite: normalizedTopLevelSite, hasCrossSiteAncestor } };
}
