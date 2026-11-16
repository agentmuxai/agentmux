// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! 256 short, common words for suggested `.vault` passphrases: each is 8 bits,
//! so eight of them are 64 bits, which scrypt then slows down guessing for.
//! The list is fixed: `suggestion` picks by index from a random byte.

pub const WORDS: [&str; 256] = [
    "able", "acid", "aged", "also", "area", "army", "atom", "aunt", "back", "bake", "ball", "band", "bank", "barn", "base", "bath",
    "bead", "beam", "bean", "bear", "beat", "bell", "belt", "bend", "bike", "bird", "boat", "bold", "bolt", "bone", "book", "boot",
    "bowl", "brew", "buck", "bulb", "bush", "cafe", "cake", "calm", "camp", "card", "care", "cart", "case", "cash", "cave", "cell",
    "chef", "chip", "city", "clay", "clip", "club", "coal", "coat", "code", "coin", "cold", "cook", "cool", "cord", "corn", "cove",
    "crab", "crew", "crop", "crow", "cube", "cure", "curl", "dairy", "damp", "dance", "dash", "dawn", "deck", "deer", "desk", "dime",
    "dish", "dive", "dock", "dome", "door", "dove", "down", "drum", "duck", "dune", "dust", "east", "echo", "edge", "elm", "ember",
    "fair", "farm", "fawn", "fern", "film", "fire", "firm", "fish", "flag", "flat", "flax", "fleet", "flint", "foam", "fold", "ford",
    "fork", "fort", "frog", "fuel", "gale", "game", "gate", "gear", "gift", "glad", "glow", "goat", "gold", "golf", "grape", "grid",
    "gulf", "hall", "hand", "harp", "hawk", "hazel", "heap", "herb", "hill", "hive", "home", "hood", "hook", "horn", "hush", "iris",
    "iron", "isle", "ivory", "jade", "jazz", "jolly", "keel", "kelp", "kind", "kite", "knit", "lake", "lamp", "lane", "lark", "leaf",
    "lemon", "lens", "lily", "lime", "link", "lion", "loft", "loom", "lunar", "lynx", "mail", "malt", "maple", "marsh", "mesa", "mint",
    "mist", "moss", "moth", "mule", "nest", "newt", "noon", "north", "oak", "oat", "ocean", "olive", "opal", "orbit", "otter", "oval",
    "pace", "palm", "pear", "pearl", "pine", "plum", "pond", "pony", "port", "quail", "quartz", "rain", "reed", "reef", "ridge", "river",
    "road", "robin", "rock", "roof", "rope", "rose", "sage", "sail", "salt", "sand", "seal", "seed", "shell", "silk", "slate", "snow",
    "soil", "song", "star", "stone", "storm", "swan", "tide", "tile", "tower", "trail", "tulip", "vale", "vine", "wave", "wheat", "wren",
    "yarn", "yew", "zest", "zinc", "amber", "birch", "cedar", "daisy", "eagle", "finch", "grove", "heron", "ibis", "juniper", "koala", "larch",
];

#[cfg(test)]
mod tests {
    use super::WORDS;

    #[test]
    fn the_list_has_256_distinct_lowercase_words() {
        let mut seen = std::collections::HashSet::new();
        for w in WORDS {
            assert!(!w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase()), "{w}");
            assert!(seen.insert(w), "{w} is listed twice");
        }
        assert_eq!(seen.len(), 256);
    }
}
