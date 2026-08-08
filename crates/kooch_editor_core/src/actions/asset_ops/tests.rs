use super::{to_pascal_case, to_snake_case};

#[test]
fn pascal_case_from_various_inputs() {
    assert_eq!(to_pascal_case("NewComponent"), "NewComponent");
    assert_eq!(to_pascal_case("player health"), "PlayerHealth");
    assert_eq!(to_pascal_case("enemy_ai"), "EnemyAi");
}

#[test]
fn snake_case_from_various_inputs() {
    assert_eq!(to_snake_case("NewSystem"), "new_system");
    assert_eq!(to_snake_case("PlayerHealth"), "player_health");
    assert_eq!(to_snake_case("enemy ai"), "enemy_ai");
}
