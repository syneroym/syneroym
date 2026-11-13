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
