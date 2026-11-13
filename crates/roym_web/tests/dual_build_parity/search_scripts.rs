use serde_json::{Value, json};
use syneroym_roym_core::services;

use super::{fixtures::*, helpers::*};

fn extract_hit_ids(res: &Value) -> Vec<String> {
    res["result"]["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["listing_id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn scenario_231_directory_finds_titles_in_every_script_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;

    // Listing 1: नल वाला (lang: hi)
    let mut p1 = full_listing_params("hi-pipe", "नल वाला");
    p1["lang"] = json!("hi");
    let (_id1, gw1, _gn1) = set_and_get(&h, p1).await;
    let (pw1, pn1) = publish_signed_listing(&h, gw1["result"]["envelope"].as_str().unwrap()).await;
    assert_eq!(pw1, pn1);
    let id1 = pw1["result"]["listing_id"].as_str().unwrap().to_string();

    // Listing 2: nal wala (lang: hi-Latn)
    let mut p2 = full_listing_params("latn-pipe", "nal wala");
    p2["lang"] = json!("hi-Latn");
    let (_id2, gw2, _gn2) = set_and_get(&h, p2).await;
    let (pw2, pn2) = publish_signed_listing(&h, gw2["result"]["envelope"].as_str().unwrap()).await;
    assert_eq!(pw2, pn2);
    let id2 = pw2["result"]["listing_id"].as_str().unwrap().to_string();

    // Listing 3: சென்னை பிளம்பர் (lang: ta)
    let mut p3 = full_listing_params("ta-plumber", "சென்னை பிளம்பர்");
    p3["lang"] = json!("ta");
    let (_id3, gw3, _gn3) = set_and_get(&h, p3).await;
    let (pw3, pn3) = publish_signed_listing(&h, gw3["result"]["envelope"].as_str().unwrap()).await;
    assert_eq!(pw3, pn3);
    let id3 = pw3["result"]["listing_id"].as_str().unwrap().to_string();

    // Listing 4: Hindi listing with an English author translation titled "Lawn
    // mowing"
    let mut p4 = full_listing_params("hi-lawn", "घास काटना");
    p4["lang"] = json!("hi");
    p4["translations"] = json!([{
        "lang": "en",
        "title": "Lawn mowing",
        "summary": "Neat lawn cutting."
    }]);
    let (_id4, gw4, _gn4) = set_and_get(&h, p4).await;
    let (pw4, pn4) = publish_signed_listing(&h, gw4["result"]["envelope"].as_str().unwrap()).await;
    assert_eq!(pw4, pn4);
    let id4 = pw4["result"]["listing_id"].as_str().unwrap().to_string();

    // Listing 5: एक नल (lang: hi)
    let mut p5 = full_listing_params("hi-one-pipe", "एक नल");
    p5["lang"] = json!("hi");
    let (_id5, gw5, _gn5) = set_and_get(&h, p5).await;
    let (pw5, pn5) = publish_signed_listing(&h, gw5["result"]["envelope"].as_str().unwrap()).await;
    assert_eq!(pw5, pn5);
    let id5 = pw5["result"]["listing_id"].as_str().unwrap().to_string();

    let mut expected_first_two = vec![id1.clone(), id2.clone()];
    expected_first_two.sort();

    // Cross-script queries for listings 1 and 2
    for query_text in ["nal wala", "नल वाला", "naal waala"] {
        let (sw, sn) = wire_invoke(
            &h,
            services::DIRECTORY,
            &env("directory.search", json!({ "text": query_text })),
        )
        .await;
        assert_eq!(stripped(&sw), stripped(&sn));
        let mut actual = extract_hit_ids(&sw);
        actual.sort();
        assert_eq!(
            actual, expected_first_two,
            "query '{query_text}' did not find listings 1 and 2"
        );
    }

    // Tamil queries for listing 3
    for query_text in ["பிளம்பர்", "senai"] {
        let (sw, sn) = wire_invoke(
            &h,
            services::DIRECTORY,
            &env("directory.search", json!({ "text": query_text })),
        )
        .await;
        assert_eq!(stripped(&sw), stripped(&sn));
        assert_eq!(
            extract_hit_ids(&sw),
            vec![id3.clone()],
            "query '{query_text}' did not find listing 3"
        );
    }

    // Author translation search
    let (sw_trans, sn_trans) =
        wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({ "text": "lawn" })))
            .await;
    assert_eq!(stripped(&sw_trans), stripped(&sn_trans));
    assert_eq!(extract_hit_ids(&sw_trans), vec![id4.clone()]);

    // Short term query "ek"
    let (sw_ek, sn_ek) =
        wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({ "text": "ek" })))
            .await;
    assert_eq!(stripped(&sw_ek), stripped(&sn_ek));
    assert_eq!(extract_hit_ids(&sw_ek), vec![id5.clone()]);
}

#[tokio::test]
async fn scenario_232_directory_accepts_a_query_in_any_script_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;

    let (sw, sn) =
        wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({ "text": "ನಲ್ಲಿ" })))
            .await;
    assert_eq!(stripped(&sw), stripped(&sn));
    assert!(sw.get("error").is_none(), "unexpected error: {sw}");
    assert_eq!(extract_hit_ids(&sw), Vec::<String>::new());
}

#[tokio::test]
async fn scenario_233_directory_refuses_more_than_16_words_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;

    let seventeen_words = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo \
                           lima mike november oscar papa quebec";
    let (sw17, sn17) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({ "text": seventeen_words })),
    )
    .await;
    assert_eq!(sw17, sn17);
    assert_eq!(sw17["error"]["code"], -32602);
    assert!(
        sw17["error"]["message"].as_str().unwrap().contains("more than 16 words"),
        "expected 'more than 16 words', got {sw17}"
    );

    let sixteen_words = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima \
                         mike november oscar papa";
    let (sw16, sn16) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.search", json!({ "text": sixteen_words })),
    )
    .await;
    assert_eq!(stripped(&sw16), stripped(&sn16));
    assert!(sw16.get("error").is_none(), "unexpected error: {sw16}");

    let (sw_punct, sn_punct) =
        wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({ "text": "!!!" })))
            .await;
    assert_eq!(sw_punct, sn_punct);
    assert_eq!(sw_punct["error"]["code"], -32602);
    assert!(
        sw_punct["error"]["message"].as_str().unwrap().contains("needs at least one word"),
        "expected 'needs at least one word', got {sw_punct}"
    );
}

#[tokio::test]
async fn scenario_235_chat_search_short_and_cross_script_parity() {
    let h = harness().await;
    let conv_id = open_conv(&h, "did:key:zPeer235").await;

    both_rpc(&h, "conversation.send", json!({ "conversation": conv_id, "body": "ek do teen" }))
        .await;

    both_rpc(&h, "conversation.send", json!({ "conversation": conv_id, "body": "एक बात" })).await;

    // Search for "ek" (short term) -> both messages match
    let (mut sw_ek, mut sn_ek) =
        both_rpc(&h, "conversation.search", json!({ "query": "ek" })).await;
    strip_volatile(&mut sw_ek);
    strip_volatile(&mut sn_ek);
    normalize_message_ids(&mut sw_ek);
    normalize_message_ids(&mut sn_ek);
    assert_eq!(sw_ek, sn_ek);
    let matches_ek = sw_ek["result"]["matches"].as_array().unwrap();
    assert_eq!(matches_ek.len(), 2);

    // Search for "बात" (cross-script long term) -> only second message matches
    let (mut sw_baat, mut sn_baat) =
        both_rpc(&h, "conversation.search", json!({ "query": "बात" })).await;
    strip_volatile(&mut sw_baat);
    strip_volatile(&mut sn_baat);
    normalize_message_ids(&mut sw_baat);
    normalize_message_ids(&mut sn_baat);
    assert_eq!(sw_baat, sn_baat);
    let matches_baat = sw_baat["result"]["matches"].as_array().unwrap();
    assert_eq!(matches_baat.len(), 1);
    assert_eq!(matches_baat[0]["body"], "एक बात");
}

#[tokio::test]
async fn scenario_234_contacts_list_text_finds_names_across_scripts_parity() {
    let h = harness().await;
    enrol_signing(&h, "profile").await;

    let c1 = json!({
        "person_did": "did:key:zRamDeva",
        "display_name": "राम",
        "conversation_address": "syneroym://addr/ram-deva",
        "favourite": false
    });
    let c2 = json!({
        "person_did": "did:key:zRamLatn",
        "display_name": "Ram",
        "conversation_address": "syneroym://addr/ram-latn",
        "favourite": false
    });
    let c3 = json!({
        "person_did": "did:key:zSita",
        "display_name": "Sita",
        "conversation_address": "syneroym://addr/sita",
        "favourite": false
    });
    let c4_name = "\u{0958}adir";
    let c4 = json!({
        "person_did": "did:key:zNfcDecomp",
        "display_name": c4_name,
        "conversation_address": "syneroym://addr/qadir"
    });
    let c5_name = "e\u{0301}lodie";
    let c5 = json!({
        "person_did": "did:key:zNfcComp",
        "display_name": c5_name,
        "conversation_address": "syneroym://addr/elodie"
    });

    for c in [&c1, &c2, &c3, &c4, &c5] {
        let (uw, un) = both_rpc(&h, "contacts.upsert", c.clone()).await;
        assert_eq!(uw, un);
        assert!(uw.get("error").is_none(), "contacts.upsert failed: {uw}");
    }

    // Search for "ram" -> Ram and राम
    let (sw_ram, sn_ram) = both_rpc(&h, "contacts.list", json!({ "text": "ram" })).await;
    assert_eq!(sw_ram, sn_ram);
    let mut names_ram: Vec<_> = sw_ram["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["display_name"].as_str().unwrap())
        .collect();
    names_ram.sort();
    assert_eq!(names_ram, vec!["Ram", "राम"]);

    // Search for "राम" -> Ram and राम
    let (sw_deva, sn_deva) = both_rpc(&h, "contacts.list", json!({ "text": "राम" })).await;
    assert_eq!(sw_deva, sn_deva);
    let mut names_deva: Vec<_> = sw_deva["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["display_name"].as_str().unwrap())
        .collect();
    names_deva.sort();
    assert_eq!(names_deva, vec!["Ram", "राम"]);

    // Favourites filter combined with text
    let (sw_fav, sn_fav) =
        both_rpc(&h, "contacts.list", json!({ "text": "sita", "favourites_only": true })).await;
    assert_eq!(sw_fav, sn_fav);
    assert_eq!(sw_fav["result"].as_array().unwrap().len(), 0);

    // Blank text returns all contacts
    let (sw_blank, sn_blank) = both_rpc(&h, "contacts.list", json!({ "text": " " })).await;
    assert_eq!(sw_blank, sn_blank);
    assert_eq!(sw_blank["result"].as_array().unwrap().len(), 5);

    // Refusal on punctuation only
    let (sw_punct, sn_punct) = both_rpc(&h, "contacts.list", json!({ "text": "!!!" })).await;
    assert_eq!(sw_punct, sn_punct);
    assert_eq!(sw_punct["error"]["code"], -32602);
    assert!(sw_punct["error"]["message"].as_str().unwrap().contains("needs at least one word"));

    // Refusal on invalid type
    let (sw_num, sn_num) = both_rpc(&h, "contacts.list", json!({ "text": 5 })).await;
    assert_eq!(sw_num, sn_num);
    assert_eq!(sw_num["error"]["code"], -32602);

    // Stored as written: contacts.list returns exact byte sequences
    let (sw_all, sn_all) = both_rpc(&h, "contacts.list", json!({})).await;
    assert_eq!(sw_all, sn_all);
    let all_contacts = sw_all["result"].as_array().unwrap();
    let c4_res = all_contacts.iter().find(|c| c["person_did"] == "did:key:zNfcDecomp").unwrap();
    assert_eq!(c4_res["display_name"], c4_name);
    let c5_res = all_contacts.iter().find(|c| c["person_did"] == "did:key:zNfcComp").unwrap();
    assert_eq!(c5_res["display_name"], c5_name);

    // profile.set / profile.get preserves exact byte sequences
    for name in ["राम", c5_name] {
        let (pw, pn) = both_rpc(
            &h,
            "profile.set",
            json!({
                "display_name": name,
                "conversation_address": "syneroym://addr/1",
                "lang": "en"
            }),
        )
        .await;
        assert_eq!(pw, pn);

        let (gw, gn) = both_rpc(&h, "profile.get", json!({})).await;
        assert_eq!(gw, gn);
        let env_str = gw["result"]["envelope"].as_str().unwrap();
        let env_val: Value = serde_json::from_str(env_str).unwrap();
        assert_eq!(env_val["payload"]["display_name"], name);
    }
}
