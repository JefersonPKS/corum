//! Esquemas das tabelas de `Data\Manager` (uma por arquivo `.cdb`).
//!
//! Fontes dos layouts: `CommonServer/BaseItem.h` (itens), `CorumOnlineProject/Effect.h` (habilidades),
//! `CorumOnlineProject/struct.h` (o resto) e `LoginAgent/ItemManager.h` (atributos e sets). Cada
//! esquema foi conferido contra o tamanho e o conteúdo do arquivo do cliente instalado; onde o
//! cliente de 2007 difere do código-fonte, o comentário diz o quê (**[hipótese]** quando o significado
//! do campo extra é só suposto).

use crate::schema::{Builder, Kind, Schema};

/// Esquema da tabela `nome` (nome do arquivo sem extensão, sem diferenciar maiúsculas).
#[must_use]
pub fn schema_for(stem: &str) -> Option<Schema> {
    let stem = stem.to_ascii_lowercase();
    let builder = Builder::new();
    let schema = match stem.as_str() {
        // ----- itens (CBaseItem: cabeçalho comum de 94 bytes + corpo por tipo) -----
        "itemweapon" => equipment_tail(
            item_head(builder)
                .u8("weapon_kind")
                .u8("hand")
                .u8("item_grade")
                .u8("class_grade")
                .u16("set_id")
                .u8("drop_box")
                .u8("race_id")
                .u8("class_id")
                .u16("weight")
                .u16("min_damage")
                .u16("max_damage")
                .u16("mana_damage")
                .u16("min_str")
                .u16("min_dex")
                .u16("min_level")
                .u16("min_slot")
                .u16("max_slot")
                .u16("base_speed")
                .u16("max_speed")
                .u16("range"),
        ),
        "itemarmor" => equipment_tail(
            item_head(builder)
                .u8("armor_kind")
                .u8("item_grade")
                .u8("class_grade")
                .u16("set_id")
                .u8("drop_box")
                .u8("race_id")
                .u8("class_id")
                .u16("weight")
                .u16("min_damage")
                .u16("max_damage")
                .u16("min_str")
                .u16("min_level")
                .u16("min_slot")
                .u16("max_slot"),
        ),
        "itemspecial" => item_tail(item_head(builder).u8("class_grade").u16("weight"))
            .u8("over_count")
            .u8("quantity_count"),
        // [hipótese] O arquivo tem 3 bytes a mais que `BASEITEM_CONSUMABLE`: um `u16` entre
        // `min_lev` e `max_lev` (sempre 0) e um `u8` no fim (0 ou 1).
        "itemconsumable" => stack_tail(item_tail(
            item_head(builder)
                .group("attr", 5, |attr| attr.u16("code").u16("value"))
                .u16("last_time")
                .u16("min_lev")
                .u16("unknown_min_lev_2")
                .u16("max_lev")
                .u16("weight"),
        ))
        .u8("unknown_tail"),
        "itemsupplies" => stack_tail(item_tail(
            item_head(builder)
                .u8("type")
                .u16("min")
                .u16("max")
                .u16("weight"),
        ))
        .u16("min_lev")
        .u16("max_lev"),
        "itemzodiac" | "item_zodiac" => stack_tail(item_tail(
            item_head(builder)
                .u16("weight")
                .u8("grade")
                .u16("min_lev")
                .u16("difficulty")
                .u32("rarity"),
        )),
        "itemride" => item_tail(item_head(builder).u8("grade").u16("weight")),
        "itemguardian" => item_tail(
            item_head(builder)
                .u8("class_grade")
                .u16("weight")
                .u8("type")
                .u16("complete_time")
                .u32("due_day_time")
                .u16("creature_id")
                .u16("die_item_id")
                .u16("broken_item_id")
                .u16("soul_item_id")
                .u16("dying_penalty")
                .u16("lv_life_up")
                .u16("base_guardian_id"),
        ),
        "itemmagicarray" => stack_tail(item_tail(
            item_head(builder)
                .u8("sub_id")
                .u8("grade")
                .u16("weight")
                .u8("class_id")
                .u8("race_id")
                .u16("min_lev")
                .u16("min_rank")
                .group("magic", 3, |magic| {
                    magic.u16("id").u16("point").u32("duration")
                }),
        )),
        "itemmaterials" => stack_tail(item_tail(
            item_head(builder)
                .u8("grade")
                .u8("order")
                .u16("power")
                .u16("bias")
                .u8("smin")
                .u8("smax")
                .u16("weight")
                .u8("lev_add")
                .u8("durability")
                .u8("difficulty"),
        )),
        "itemmixupgrade" => stack_tail(item_tail(
            item_head(builder)
                .u8("grade")
                .u16("weight")
                .u16("min_lev")
                .u16("durability")
                .u16("difficulty"),
        )),
        "itemmagicfieldarray" => stack_tail(item_tail(
            item_head(builder)
                .u16("weight")
                .u16("min_lv")
                .group("value", 5, |value| {
                    value.u16("id").u8("who").i16("value").u8("formula")
                }),
        )),
        "itemupgrade" | "item_upgrade" => stack_tail(item_tail(
            item_head(builder)
                .u16("weight")
                .u8("classification")
                .u16("probability_minus")
                .u16("r1_plus")
                .u16("r2_plus")
                .u16("w_grade")
                .array("formula", 18, Kind::U16),
        )),
        "itemliquid" => stack_tail(item_tail(
            item_head(builder)
                .u16("weight")
                .u8("classification")
                .u16("slot_1")
                .u16("fluent_a")
                .u16("slot_2")
                .u16("fluent_b")
                .u16("fluent_c")
                .u16("w_grade"),
        )),
        "itemedition" => stack_tail(item_tail(
            item_head(builder)
                .u16("weight")
                .u16("option")
                .u16("probability_plus")
                .u16("liquid_a_min")
                .u16("liquid_a_max")
                .u16("liquid_b_min")
                .u16("liquid_b_max")
                .array("formula", 18, Kind::U16),
        )),
        // [hipótese] O arquivo tem 4 bytes a mais que `BASEITEM_BAG` no fim (sempre 0).
        "itembag" => item_tail(
            item_head(builder)
                .u8("type")
                .u16("value_min")
                .u16("value_max")
                .u16("min_lev"),
        )
        .hex("unknown_tail", 4),
        // Não há struct no código-fonte: cabeçalho de item + 162 bytes ainda não decifrados.
        "itemtalisman" => item_head(builder).hex("unknown_body", 162),
        "itemsetinfo" => builder
            .u16("id")
            .text("name_kor", 50)
            .text("name_eng", 50)
            .u8("full_set")
            .array("set_id", 7, Kind::U16)
            .group("bonus", 10, |bonus| bonus.u16("kind").u16("value"))
            .group("full_bonus", 8, |bonus| bonus.u16("kind").u16("value")),
        // [hipótese] receitas: id + 3 campos de 16 bits (dois parecem ids de item).
        "itemmaking" => builder
            .u16("id")
            .u16("field_1")
            .u16("field_2")
            .u16("field_3"),
        // [hipótese] o último byte (0 ou 1) não existe na struct do código-fonte; o texto tem 100 bytes.
        "itemattrdefine" => builder
            .u32("id")
            .u16("formula")
            .u16("success_formula")
            .u8("skill_id")
            .u8("value_type")
            .text("description", 100)
            .u8("unknown_flag"),
        "itemattrvaluelist" => builder
            .u32("id")
            .u8("value_type")
            .i16("min")
            .i16("max")
            .u8("pbt"),
        "itemresource" => builder
            .u16("id")
            .text("icon_file", 40)
            .text("model_file", 40)
            .u8("icon_start_index")
            .u8("icon_count")
            .u16("model_count")
            .u8("resource_type")
            .u8("preload")
            .u8("animation"),
        "itemoption" => builder
            .u16("id")
            .u8("count")
            .array("display_flag", 4, Kind::U8)
            .array("option", 4, Kind::Text(64)),
        "itemstore" => builder.u16("item_id").u8("kind").u16("map_id"),

        // ----- habilidades (BASESKILL) -----
        "skill" | "skilleffect" => builder
            .u8("id")
            .u32("resource_id")
            .u32("status_resource_id")
            .u16("property")
            .u8("type")
            .u8("able_class")
            .text("name", 50)
            .text("description", 255)
            .text("description2_1", 64)
            .text("description2_2", 64)
            .u8("skill_target")
            .u8("skill_type")
            .u32("range")
            .u32("min_mastery")
            .u32("casting_time")
            .u32("cool_time")
            .u32("effect_position")
            .u8("joint_effect")
            .u8("formula")
            .u8("success_formula")
            .array("sound", 8, Kind::U16)
            .group("status", 5, |status| status.u16("id").u8("formula"))
            .u8("crime")
            .u8("efficiency")
            .group("level", 51, |level| {
                level
                    .i32("min")
                    .i32("max")
                    .i32("mana")
                    .i32("compass")
                    .i32("duration")
                    .i32("probability")
            }),
        "skillresource" => builder
            .u16("id")
            .u8("skill_type")
            .text("icon_file", 20)
            .u16("icon_index")
            .text("icon_file_act", 20)
            .u16("icon_index_act")
            .u8("kind")
            .u8("kind_index")
            .u8("kind_position"),
        "cptable" => builder
            .u16("id")
            .text("name_kor", 50)
            .text("name_eng", 35)
            .text("description", 128)
            .u8("class")
            .u16("rate")
            .u16("animation_1")
            .u16("animation_2")
            .u16("sound_1")
            .u16("sound_2")
            .u16("apply_time")
            .u8("party_use")
            .group("value", 5, |value| value.u16("id").u16("value")),

        // ----- jogador, NPC, mundo e interface -----
        // O `SLEVEL_EXP` do código-fonte (5 bytes) não bate com este arquivo; vale o arquivo (9 bytes).
        "level" => builder.u8("level").u64("exp"),
        "guardianlevel" | "guardianexp" => builder.u8("level").u32("exp"),
        "baseclassinfo" => builder
            .i32("max_aura")
            .i32("max_divine")
            .i32("max_summon")
            .i32("max_chakra")
            .i32("max_magic"),
        "npctable" => builder
            .u32("id")
            .text("name", 32)
            .u32("type")
            .text("message1", 256)
            .text("message2", 256)
            .text("message3", 256),
        "dungeonproductionitemminmax" => builder
            .u8("id")
            .u16("item_id_min")
            .u16("item_id_max")
            .u16("item_id_default"),
        "help" | "helpinfo" => builder
            .u16("id")
            .text("text", 64)
            .u16("left")
            .u16("top")
            .u16("link_text_id")
            .u8("kind"),
        "keyinfo" => builder.i32("index").text("info", 128),
        "groupinfo" => builder
            .u8("type")
            .u8("level")
            .array("member", 5, Kind::U16)
            .u32("exp")
            .u8("aura")
            .u8("make_size"),
        _ => return None,
    };
    Some(schema.build())
}

/// Cabeçalho comum de `CBaseItem` (94 bytes). No cliente de 2007 o campo "coreano" traz o nome em inglês.
fn item_head(builder: Builder) -> Builder {
    builder
        .u16("id")
        .text("name_kor", 50)
        .text("name_eng", 35)
        .u32("code_id")
        .u8("code_type")
        .u8("rand_item")
        .u8("movable")
}

/// Fim comum dos corpos: preço, venda e ícone/modelo do inventário.
fn item_tail(builder: Builder) -> Builder {
    builder
        .u32("price")
        .u32("sell_price")
        .u8("iv_icon_count")
        .u8("iv_index")
        .u8("mod_count")
}

/// Fim comum de armas e armaduras: opções de set, opções de parte, quantidades, material e preço.
fn equipment_tail(builder: Builder) -> Builder {
    item_tail(
        builder
            .group("set_option", 6, |option| {
                option.u16("kind").u16("min").u16("max")
            })
            .group("part_option", 4, |option| option.u16("kind").u16("value"))
            .u16("m_qtt")
            .u16("g_qtt")
            .u16("w_qtt")
            .u16("l_qtt")
            .u8("material"),
    )
}

fn stack_tail(builder: Builder) -> Builder {
    builder.u8("over_count").u8("quantity_count")
}

/// Arquivos `.cdb` (sem extensão) que têm esquema, em ordem alfabética.
pub const SCHEMA_FILES: &[&str] = &[
    "BaseClassInfo",
    "CPTable",
    "DungeonProductionItemMinMax",
    "GroupInfo",
    "GuardianExp",
    "GuardianLevel",
    "Help",
    "helpinfo",
    "ItemArmor",
    "ItemAttrDefine",
    "ItemAttrValueList",
    "ItemBag",
    "ItemConsumable",
    "ItemEdition",
    "ItemGuardian",
    "ItemLiquid",
    "ItemMagicArray",
    "ItemMagicFieldArray",
    "ItemMaking",
    "ItemMaterials",
    "ItemMixUpgrade",
    "itemoption",
    "itemresource",
    "ItemRide",
    "ItemSetInfo",
    "ItemSpecial",
    "itemstore",
    "ItemSupplies",
    "Itemtalisman",
    "ItemUpgrade",
    "Item_Upgrade",
    "ItemWeapon",
    "Item_Zodiac",
    "itemzodiac",
    "KeyInfo",
    "Level",
    "npctable",
    "Skill",
    "SkillEffect",
    "SkillResource",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_file_has_a_schema() {
        for name in SCHEMA_FILES {
            assert!(schema_for(name).is_some(), "{name}");
        }
        assert!(schema_for("questlist").is_none());
    }

    /// Com `CORUM_DATA` (pasta `Data` do cliente): todo `.cdb` com esquema tem de fechar em número
    /// exato de registros e voltar idêntico ao original depois de `.cdb` → TSV → `.cdb`.
    #[test]
    fn every_real_table_round_trips_through_tsv() {
        let Some(data) = std::env::var_os("CORUM_DATA") else {
            return;
        };
        let manager = std::path::Path::new(&data).join("Manager");
        let mut failures = Vec::new();
        for name in SCHEMA_FILES {
            let path = manager.join(format!("{name}.cdb"));
            let original = std::fs::read(&path).unwrap_or_else(|_| panic!("{}", path.display()));
            let schema = schema_for(name).unwrap();
            let body = crate::cdb::decode(&original).unwrap();
            let outcome = schema
                .to_tsv(&body)
                .and_then(|tsv| schema.from_tsv(&tsv))
                .map(|again| crate::cdb::encode(&again));
            match outcome {
                Ok(bytes) if bytes == original => {}
                Ok(_) => failures.push(format!("{name}: bytes differ after the round trip")),
                Err(error) => failures.push(format!("{name}: {error}")),
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn record_sizes_match_the_source_structs() {
        let size = |name: &str| schema_for(name).unwrap().record_size();
        // sizeof(CBaseItem) header 94 + corpo do `BaseItem.h`.
        assert_eq!(size("ItemWeapon"), 94 + 105);
        assert_eq!(size("ItemArmor"), 94 + 94);
        assert_eq!(size("ItemSpecial"), 94 + 16);
        assert_eq!(size("ItemSupplies"), 94 + 24);
        assert_eq!(size("ItemSetInfo"), 189);
        assert_eq!(size("Skill"), 1728);
        assert_eq!(size("npctable"), 808);
        assert_eq!(size("CPTable"), 249);
        assert_eq!(size("itemoption"), 263);
        assert_eq!(size("itemresource"), 89);
    }
}
