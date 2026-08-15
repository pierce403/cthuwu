import { getAddress, getBytes, keccak256, toUtf8Bytes } from "ethers";

/** Canonical V1 Branding custom-trait key. The current NFT owner controls this public value. */
export const ACOLYTE_NAME_TRAIT = "Acolyte Name";
export const ACOLYTE_NAME_SCHEME = "acolyte-v1";

const FIRST = [
  "Ainsworth", "Ashcombe", "Bellingham", "Blackwood", "Cavendish", "Cholmondeley",
  "Davenport", "Devereux", "Eversleigh", "Fairfax", "Featherstone", "Fitzwilliam",
  "Fortescue", "Gainsborough", "Harrington", "Hawthorne", "Kensington", "Langford",
  "Marlborough", "Montague", "Pemberton", "Ravenscroft", "Sinclair", "Somerset",
  "Stanhope", "Thackeray", "Wainwright", "Weatherby", "Wellington", "Westcott",
  "Whitcombe", "Winchester",
  "Abberley", "Adderley", "Alvingham", "Bancroft", "Barrington", "Beauchamp",
  "Beresford", "Brabazon", "Broughton", "Buckhurst", "Cadogan", "Chatterton",
  "Chetwynd", "Coleridge", "Digby", "Edgeworth", "Frobisher", "Granville",
  "Hardwick", "Hesketh", "Lascelles", "Mandeville", "Mortimer", "Neville", "Paget",
  "Rawdon", "Rockingham", "Sherborne", "Trelawney", "Waldegrave", "Wentworth", "Wyndham",
] as const;

const SECOND = [
  "Arbuthnot", "Bramwell", "Carrington", "Chadwick", "Clavering", "Cumberland",
  "Darlington", "Ellsworth", "Farnsworth", "Fetherstonhaugh", "Godolphin", "Grantham",
  "Hargreaves", "Kingsley", "Loxley", "Marchbanks", "Molesworth", "Northcott",
  "Ormsby", "Ponsonby", "Radcliffe", "Sackville", "Smythe", "Tavistock", "Templeton",
  "Uxbridge", "Vane", "Walsingham", "Wetherell", "Whittington", "Wickham", "Worthing",
  "Acton", "Blandford", "Boswell", "Bridgeman", "Bulwer", "Calthorpe", "Chichester",
  "Coningsby", "Delamere", "Denham", "Dorrington", "Eddington", "Fane", "Fitzalan",
  "Grafton", "Grosvenor", "Harcourt", "Ingleby", "Jermyn", "Kettering", "Lowther",
  "Marwood", "Painswick", "Quenby", "Rivington", "SaintJohn", "Strathmore", "Tichborne",
  "Underhill", "Vernon", "Wrottesley", "Yelverton",
] as const;

const ESTATE_PREFIX = [
  "Alder", "Amber", "Apple", "Ash", "Barrow", "Beech", "Bel", "Birch", "Black",
  "Blen", "Blythe", "Bracken", "Bram", "Briar", "Bright", "Broad", "Buck", "Cedar",
  "Charn", "Clear", "Cold", "Crow", "Deep", "Dun", "East", "Elder", "Elm", "Ever",
  "Fair", "Fern", "Fleet", "Fox", "Glen", "Gold", "Grand", "Green", "Grey", "Hart",
  "Hazel", "High", "Holly", "Honey", "Ivy", "Kings", "Lang", "Little", "Long", "Low",
  "Maple", "Marsh", "Mere", "Mill", "Nether", "North", "Oak", "Pen", "Pine", "Raven",
  "Red", "Rose", "Silver", "South", "Stan", "Wych",
] as const;

const ESTATE_SUFFIX = [
  "abbey", "bank", "borough", "bourne", "bridge", "brook", "bury", "castle", "chester",
  "cliff", "combe", "court", "croft", "dale", "den", "field", "ford", "gate", "grove",
  "hall", "ham", "haven", "heath", "hill", "holm", "hurst", "ington", "land", "leigh",
  "manor", "marsh", "meadow", "mere", "mill", "minster", "moor", "mount", "park", "pool",
  "port", "ridge", "rose", "stead", "stoke", "stone", "thorp", "ton", "vale", "view",
  "ville", "wall", "water", "way", "well", "wick", "wood", "worth", "yard", "end", "fen",
  "green", "lodge", "priory", "quay",
] as const;

/**
 * Gives a random browser EOA a stable, version-1 surname without storing another secret.
 * Changing these lists would rename existing Acolytes, so future schemes need a new version.
 */
export function acolyteName(address: string): string {
  const canonical = getAddress(address);
  const digest = getBytes(keccak256(canonical));
  const first = index(digest[0]!, digest[1]!, FIRST.length);
  const second = index(digest[2]!, digest[3]!, SECOND.length);
  const estatePrefix = index(digest[4]!, digest[5]!, ESTATE_PREFIX.length);
  const estateSuffix = index(digest[6]!, digest[7]!, ESTATE_SUFFIX.length);
  return `${FIRST[first]}-${SECOND[second]} of ${ESTATE_PREFIX[estatePrefix]}${ESTATE_SUFFIX[estateSuffix]}`;
}

/** Test/audit fingerprint: table changes require a new scheme and migration, never a silent rename. */
export function acolyteNameTableFingerprint(): string {
  return keccak256(toUtf8Bytes([
    ACOLYTE_NAME_SCHEME,
    FIRST.join("\u0000"),
    SECOND.join("\u0000"),
    ESTATE_PREFIX.join("\u0000"),
    ESTATE_SUFFIX.join("\u0000"),
  ].join("\u0001")));
}

export function acolyteNameSpaceSize(): number {
  return FIRST.length * SECOND.length * ESTATE_PREFIX.length * ESTATE_SUFFIX.length;
}

export function nftAcolyteName(
  traits: readonly { traitType: string; value: string }[],
): string | undefined {
  const value = traits.find((trait) => trait.traitType === ACOLYTE_NAME_TRAIT)?.value.trim();
  return value || undefined;
}

function index(high: number, low: number, length: number): number {
  return ((high << 8) | low) % length;
}
