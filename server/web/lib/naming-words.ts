/**
 * Vocabulary for human-friendly upload codes.
 *
 * Two independent concerns live here, both deliberately kept out of
 * lib/naming.ts so the generator stays readable:
 *
 *  1. `ADJECTIVES` / `NOUNS` — the Gyazo-style word pool (`funny-cat`,
 *     `ineffable-guitar`).
 *  2. `UNAMBIGUOUS_ALPHABET` — the character set for random codes.
 */

/**
 * Characters that survive being read aloud, written by hand, or retyped from a
 * screenshot.
 *
 * Removed on purpose: `0`/`O`/`o`, `1`/`l`/`I`. Those are the pairs people
 * actually get wrong when someone dictates a link over the phone or copies it
 * off a monitor. Everything else stays, which keeps the pool at 56 characters —
 * a 5-character code still has ~550 million possibilities.
 */
export const UNAMBIGUOUS_ALPHABET =
  "23456789" +
  "abcdefghijkmnpqrstuvwxyz" + // no l, no o
  "ABCDEFGHJKLMNPQRSTUVWXYZ"; // no I, no O

/** Distinct codes available at a given length, for the "pool exhausted" warning. */
export function poolSize(length: number): number {
  return Math.pow(UNAMBIGUOUS_ALPHABET.length, Math.max(0, length));
}

/**
 * Adjectives: short, positive-or-neutral, easy to spell. Nothing that reads as
 * an insult when paired with a noun — these end up in links people share.
 */
export const ADJECTIVES = [
  "able", "adept", "agile", "airy", "amber", "ample", "arctic", "ashen",
  "autumn", "avid", "balmy", "bold", "brave", "brief", "bright", "brisk",
  "calm", "candid", "cheery", "chill", "civic", "clear", "clever", "cobalt",
  "cosmic", "cozy", "crimson", "crisp", "curious", "daring", "dawn", "deft",
  "dense", "dewy", "dizzy", "downy", "dreamy", "dusky", "eager", "early",
  "earthy", "easy", "elated", "elder", "electric", "elegant", "empty", "epic",
  "even", "exact", "fabled", "faint", "fair", "famous", "fancy", "feisty",
  "fiery", "fine", "flat", "fleet", "floral", "fluent", "fluffy", "fond",
  "frank", "fresh", "frosty", "funny", "fuzzy", "gentle", "giddy", "gilded",
  "glad", "gleaming", "glossy", "golden", "grand", "grassy", "green", "hardy",
  "hazel", "hazy", "hearty", "hidden", "holy", "honest", "humble", "icy",
  "ideal", "idle", "indigo", "ineffable", "inner", "iron", "ivory", "jade",
  "jolly", "jovial", "joyful", "keen", "kind", "lacy", "late", "lawful",
  "lazy", "level", "light", "lilac", "lively", "loyal", "lucid", "lucky",
  "lunar", "lush", "magic", "main", "major", "maple", "marble", "meek",
  "mellow", "merry", "mighty", "mild", "minty", "misty", "modest", "murky",
  "narrow", "navy", "neat", "nimble", "noble", "north", "novel", "oaken",
  "olive", "opal", "orange", "outer", "pale", "patient", "peachy", "pearly",
  "perky", "plain", "plucky", "polar", "polite", "prime", "proud", "pure",
  "purple", "quaint", "quick", "quiet", "rapid", "rare", "ready", "regal",
  "rich", "rising", "rosy", "royal", "ruby", "rugged", "rustic", "sage",
  "salty", "sandy", "scarlet", "secret", "serene", "shady", "sharp", "sheer",
  "shiny", "silent", "silky", "silver", "simple", "sleek", "slender", "smart",
  "smooth", "snowy", "soft", "solar", "solid", "sonic", "sound", "south",
  "spare", "spry", "stark", "steady", "steep", "stellar", "still", "stormy",
  "sturdy", "sunny", "sunset", "swift", "tame", "tender", "thrifty", "tidy",
  "timber", "tiny", "trusty", "twilight", "upbeat", "urban", "usual", "valiant",
  "velvet", "vernal", "vested", "vital", "vivid", "warm", "wavy", "wayward",
  "wealthy", "west", "whole", "wild", "windy", "winter", "wise", "witty",
  "woven", "young", "zany", "zesty",
] as const;

/**
 * Nouns: concrete, everyday things. Concrete beats abstract — `amber-otter` is
 * easier to remember and repeat than `amber-notion`.
 */
export const NOUNS = [
  "acorn", "album", "alley", "anchor", "apple", "arbor", "arch", "arrow",
  "aspen", "atlas", "aurora", "badger", "bagel", "bamboo", "banjo", "barley",
  "basin", "bason", "beacon", "beagle", "bean", "bear", "beaver", "bell",
  "birch", "bison", "blossom", "bluff", "boat", "bonsai", "borough", "bottle",
  "boulder", "branch", "brass", "bread", "breeze", "bridge", "brook", "brush",
  "burrow", "cabin", "cable", "cactus", "camera", "candle", "canoe", "canvas",
  "canyon", "cargo", "carrot", "castle", "cedar", "cello", "chalk", "cherry",
  "chime", "cider", "cinder", "circus", "clover", "cobble", "cocoa", "comet",
  "compass", "coral", "cottage", "cotton", "crane", "crater", "crayon", "creek",
  "crown", "cypress", "daisy", "dawn", "delta", "denim", "desert", "diamond",
  "dolphin", "domain", "dove", "dragon", "drum", "dune", "eagle", "echo",
  "ember", "engine", "estate", "falcon", "fable", "fern", "ferry", "fiddle",
  "finch", "flame", "flint", "flower", "flute", "forest", "fossil", "fountain",
  "fox", "galaxy", "garden", "gazelle", "geyser", "ginger", "glacier", "glade",
  "granite", "grotto", "grove", "guitar", "hamlet", "harbor", "harp", "harvest",
  "hawk", "hazel", "heron", "hickory", "hollow", "honey", "horizon", "island",
  "ivy", "jasmine", "jungle", "juniper", "kayak", "kettle", "keystone", "lagoon",
  "lantern", "laurel", "leaf", "ledger", "lemon", "lily", "linden", "lion",
  "lodge", "lotus", "lynx", "magnet", "mallard", "mango", "manor", "maple",
  "marble", "marina", "meadow", "medley", "mesa", "meteor", "mirror", "monsoon",
  "moss", "mountain", "mural", "nectar", "nest", "nettle", "nickel", "nutmeg",
  "oasis", "oat", "ocean", "octave", "onyx", "opal", "orbit", "orchard",
  "orchid", "osprey", "otter", "outpost", "owl", "oyster", "pagoda", "palace",
  "palm", "panda", "pantry", "parade", "parcel", "parlor", "pasture", "peak",
  "pebble", "pelican", "pepper", "petal", "pewter", "phoenix", "piano", "pigeon",
  "pillar", "pine", "planet", "plateau", "plaza", "plum", "pocket", "pollen",
  "pond", "poplar", "poppy", "portal", "prairie", "puffin", "pumpkin", "quarry",
  "quartz", "quilt", "rabbit", "radish", "rain", "rapids", "raven", "reef",
  "ribbon", "ridge", "river", "robin", "rocket", "rooster", "rope", "rosemary",
  "rudder", "saddle", "sage", "sail", "salmon", "sandbar", "sapling", "satchel",
  "scarab", "seagull", "shadow", "shell", "shore", "signal", "silo", "sky",
  "slate", "sled", "sloth", "snapper", "sparrow", "spruce", "squirrel", "stable",
  "stag", "station", "stone", "stream", "summit", "sundial", "sunset", "swallow",
  "swan", "table", "tandem", "teapot", "temple", "terrace", "thistle", "thorn",
  "thunder", "tiger", "timber", "toucan", "tower", "trail", "tulip", "tundra",
  "turtle", "valley", "vessel", "village", "vineyard", "violet", "vista", "walnut",
  "warbler", "waterfall", "wave", "willow", "window", "wombat", "wren", "yarrow",
  "zebra", "zenith", "zephyr",
] as const;

/** Two-word pairs available before the generator has to reach for a suffix. */
export const WORD_PAIR_POOL = ADJECTIVES.length * NOUNS.length;
