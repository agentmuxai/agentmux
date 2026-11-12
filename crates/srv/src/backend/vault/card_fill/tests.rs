use super::*;

fn parts(pairs: &[(&str, &str)]) -> Parts {
    Parts(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
}

fn visa() -> Parts {
    parts(&[("number", "4111111111111111"), ("exp_month", "03"), ("exp_year", "2027"), ("cvc", "123"), ("name", "Alex Doe"), ("zip", "94107")])
}

fn field(slot: &str) -> FormField {
    FormField { slot: slot.into(), ..Default::default() }
}

fn text(s: Shaped) -> String {
    match s {
        Shaped::Text(t) | Shaped::Choose(t) => t.to_string(),
        Shaped::Left(why) => panic!("left empty: {why}"),
    }
}

fn left_why(s: Shaped) -> String {
    match s {
        Shaped::Left(why) => why,
        _ => panic!("expected a field left empty"),
    }
}

fn expiry(max_length: Option<usize>, placeholder: &str, pattern: &str) -> String {
    let f = FormField { slot: "card-expiry".into(), max_length, placeholder: placeholder.into(), pattern: pattern.into(), ..Default::default() };
    text(shape(&f, &visa()))
}

#[test]
fn a_single_expiry_field_takes_the_shape_it_asks_for() {
    assert_eq!(expiry(None, "", ""), "03/27");
    assert_eq!(expiry(Some(5), "MM/YY", ""), "03/27");
    assert_eq!(expiry(None, "MM / YY", ""), "03/27");
    assert_eq!(expiry(None, "MM/YYYY", ""), "03/2027");
    assert_eq!(expiry(Some(7), "", ""), "03/2027");
    assert_eq!(expiry(Some(4), "", ""), "0327");
    assert_eq!(expiry(Some(6), "", ""), "032027");
    // A field's length decides before a pattern: four characters never get a four-digit year.
    assert_eq!(expiry(Some(4), "", r"[0-9]{4}"), "0327");
    assert_eq!(expiry(Some(5), "MM/YYYY", ""), "03/27");
    // A pattern alone: a bare four digits is MMYY, six is MMYYYY, a separator before four is MM/YYYY.
    assert_eq!(expiry(None, "", r"\d{4}"), "0327");
    assert_eq!(expiry(None, "", "[0-9]{4}"), "0327");
    assert_eq!(expiry(None, "", r"\d{6}"), "032027");
    assert_eq!(expiry(None, "", r"\d{2}/\d{4}"), "03/2027");
    assert_eq!(expiry(None, "MMYY", ""), "0327");
    assert_eq!(expiry(None, "MM-YY", ""), "03-27");
    assert_eq!(expiry(None, "MM YY", ""), "03 27");
    // A placeholder that is words says nothing about the separator.
    assert_eq!(expiry(None, "Expiration date", ""), "03/27");
    assert_eq!(expiry(None, "Exp. date", ""), "03/27");
    assert_eq!(expiry(None, "Valid thru", ""), "03/27");
    assert_eq!(expiry(Some(5), "Expiry date", ""), "03/27");
    assert_eq!(expiry(None, "", r"\d{2}/\d{4}"), "03/2027");
}

#[test]
fn month_and_year_fields_take_text_or_a_choice() {
    assert_eq!(text(shape(&field("card-exp-month"), &visa())), "03");
    assert_eq!(text(shape(&field("card-exp-year"), &visa())), "2027");
    let short = FormField { slot: "card-exp-year".into(), max_length: Some(2), ..Default::default() };
    assert_eq!(text(shape(&short, &visa())), "27");
    let yy = FormField { slot: "card-exp-year".into(), placeholder: "YY".into(), ..Default::default() };
    assert_eq!(text(shape(&yy, &visa())), "27");

    let select = |slot: &str| FormField { slot: slot.into(), select: true, ..Default::default() };
    assert!(matches!(shape(&select("card-exp-month"), &visa()), Shaped::Choose(m) if *m == "03"));
    assert!(matches!(shape(&select("card-exp-year"), &visa()), Shaped::Choose(y) if *y == "2027"));
}

#[test]
fn an_older_card_with_one_expiry_part_still_fills_each_field() {
    let old = parts(&[("number", "4111111111111111"), ("expiry", "03/27")]);
    assert_eq!(text(shape(&field("card-exp-month"), &old)), "03");
    assert_eq!(text(shape(&field("card-exp-year"), &old)), "2027");
    assert_eq!(text(shape(&field("card-expiry"), &old)), "03/27");
}

#[test]
fn the_number_is_digits_unless_the_field_has_room_for_the_groups() {
    assert_eq!(text(shape(&field("card-number"), &visa())), "4111111111111111");
    let roomy = FormField { slot: "card-number".into(), max_length: Some(19), placeholder: "1234 5678 9012 3456".into(), ..Default::default() };
    assert_eq!(text(shape(&roomy, &visa())), "4111 1111 1111 1111");
    let tight = FormField { slot: "card-number".into(), max_length: Some(16), placeholder: "1234 5678 9012 3456".into(), ..Default::default() };
    assert_eq!(text(shape(&tight, &visa())), "4111111111111111");
    // Words in a placeholder are not digit groups, and a digits-only pattern takes no spaces.
    let words = FormField { slot: "card-number".into(), max_length: Some(19), placeholder: "Card number".into(), ..Default::default() };
    assert_eq!(text(shape(&words, &visa())), "4111111111111111");
    let digits_only = FormField { slot: "card-number".into(), max_length: Some(19), placeholder: "1234 5678 9012 3456".into(), pattern: "[0-9]*".into(), ..Default::default() };
    assert_eq!(text(shape(&digits_only, &visa())), "4111111111111111");
    let short = FormField { slot: "card-number".into(), max_length: Some(12), ..Default::default() };
    assert!(left_why(shape(&short, &visa())).starts_with("this field takes 12 characters"));
}

#[test]
fn a_code_field_shorter_than_the_code_is_left_empty_and_says_so_without_the_code() {
    let amex = parts(&[("number", "378282246310005"), ("cvc", "1234")]);
    let three = FormField { slot: "card-cvc".into(), max_length: Some(3), ..Default::default() };
    let why = left_why(shape(&three, &amex));
    assert_eq!(why, "this field takes 3 digits and the card's code has 4");
    assert!(!why.contains("1234"));
    assert_eq!(text(shape(&field("card-cvc"), &amex)), "1234");
    assert_eq!(text(shape(&field("card-cvc"), &visa())), "123");
}

#[test]
fn a_part_the_card_lacks_is_left_for_the_person_by_name() {
    let bare = parts(&[("number", "4111111111111111")]);
    assert_eq!(left_why(shape(&field("card-cvc"), &bare)), "the card has no security code saved");
    assert_eq!(left_why(shape(&field("card-zip"), &bare)), "the card has no billing ZIP saved");
    assert_eq!(left_why(shape(&field("card-name"), &bare)), "the card has no holder name saved");
    assert_eq!(left_why(shape(&field("card-expiry"), &bare)), "the card has no expiry saved");
    assert_eq!(text(shape(&field("card-zip"), &visa())), "94107");
    assert_eq!(text(shape(&field("card-name"), &visa())), "Alex Doe");
}

#[test]
fn every_card_slot_has_a_part_and_a_name() {
    for slot in ["card-number", "card-expiry", "card-exp-month", "card-exp-year", "card-cvc", "card-name", "card-zip"] {
        assert!(part_of_slot(slot).is_some(), "{slot}");
        assert_ne!(part_name(slot), "card detail", "{slot}");
    }
    assert!(part_of_slot("tax-id").is_none());
}
