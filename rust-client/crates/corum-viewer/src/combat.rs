//! Regras do combate offline do sandbox: números de movimento do cliente original, vida, alcance,
//! decisões do monstro. Não conhece gráficos: só recebe distâncias e tempos, para ser testável.
//!
//! **Os números de dano, vida e alcance são de brinquedo [hipótese]**: servem para ter um combate para
//! testar as animações; o jogo de verdade (fórmulas, tabelas de monstros, `BaseMonsterInfo`) vem do
//! servidor. Já os **números de movimento** e o tipo de arma são os do cliente original
//! (`GameDefine.h`, `ITEM_DISTRIBUTE` em `ItemManagerDefine.h`).

/// Tipos de movimento do jogador (`MOTION_TYPE_*` de `GameDefine.h`); os que o sandbox ainda não
/// usa ficam documentados aqui.
#[allow(dead_code)]
pub mod player_motion {
    pub const STAND1: u16 = 4;
    pub const WALK: u16 = 7;
    pub const RUN: u16 = 8;
    pub const ATTACK1_1: u16 = 9;
    pub const DEFENSEFAIL: u16 = 30;
    pub const DYING: u16 = 33;
}

/// Tipos de movimento do monstro (`MON_MOTION_TYPE_*`), que são o slot do `.chr` mais 1.
#[allow(dead_code)]
pub mod mob_motion {
    pub const STAND1: u16 = 1;
    pub const MOVE1: u16 = 3;
    pub const ATTACK1: u16 = 5;
    pub const DEFENSEFAIL1: u16 = 12;
    pub const DOWN: u16 = 15;
}

/// Cada faixa de `ITEM_DISTRIBUTE` ids de arma é um tipo de arma (`m_byItemType`); o tipo 0 é sem arma.
pub const ITEM_DISTRIBUTE: u16 = 200;

pub const PLAYER_MAX_HP: i32 = 100;
pub const MOB_MAX_HP: i32 = 60;
/// Dano de um golpe do jogador (sorteado nesta faixa) e do monstro.
pub const PLAYER_DAMAGE: (i32, i32) = (8, 12);
pub const MOB_DAMAGE: i32 = 6;
/// Alcance de ataque em tiles (centro a centro).
pub const PLAYER_REACH: f32 = 1.5;
pub const MOB_REACH: f32 = 1.4;
/// O monstro passa a perseguir dentro de `AGGRO_RANGE` e desiste além de `LEASH_RANGE`.
pub const AGGRO_RANGE: f32 = 5.0;
pub const LEASH_RANGE: f32 = 10.0;
/// Pausa do monstro entre um golpe e o seguinte, em segundos.
pub const MOB_ATTACK_COOLDOWN: f32 = 0.7;
/// Tempo do monstro caído até reaparecer, e do jogador caído.
pub const MOB_RESPAWN_SECONDS: f32 = 4.0;
pub const PLAYER_RESPAWN_SECONDS: f32 = 3.0;
/// Duração da reação ao dano quando o movimento não existe.
pub const HURT_SECONDS: f32 = 0.5;
/// Velocidade de perseguição do monstro, em tiles por segundo.
pub const MOB_CHASE_SPEED: f32 = 2.4;

/// Tipo de arma da mão direita (`m_byItemType = id / ITEM_DISTRIBUTE + 1`); 0 sem arma.
#[must_use]
pub fn item_type(weapon_id: u16) -> u16 {
    if weapon_id == 0 {
        0
    } else {
        weapon_id / ITEM_DISTRIBUTE + 1
    }
}

/// Slot do `.chr` de um jogador: `tipo_de_arma × 50 + movimento − 1` (`SetAction` do cliente).
#[must_use]
pub fn player_slot(item_type: u16, motion: u16) -> usize {
    usize::from(item_type) * 50 + usize::from(motion) - 1
}

/// Slot do `.chr` de um monstro: uma entrada por `MON_MOTION_TYPE`.
#[must_use]
pub fn mob_slot(motion: u16) -> usize {
    usize::from(motion) - 1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vitals {
    pub hp: i32,
    pub max: i32,
}

impl Vitals {
    #[must_use]
    pub fn full(max: i32) -> Self {
        Self { hp: max, max }
    }

    #[must_use]
    pub fn alive(self) -> bool {
        self.hp > 0
    }

    /// Fração de vida, de 0 a 1 (a barra sobre a cabeça).
    #[must_use]
    pub fn fraction(self) -> f32 {
        (self.hp.max(0) as f32 / self.max.max(1) as f32).clamp(0.0, 1.0)
    }

    /// Tira vida e diz se este golpe matou.
    pub fn hurt(&mut self, amount: i32) -> bool {
        let was_alive = self.alive();
        self.hp = (self.hp - amount.max(0)).max(0);
        was_alive && !self.alive()
    }
}

/// Dano de um golpe do jogador a partir de um número qualquer (a contagem de golpes): determinístico
/// para os testes e sem precisar de gerador aleatório.
#[must_use]
pub fn player_damage_roll(sequence: u32) -> i32 {
    let (low, high) = PLAYER_DAMAGE;
    let span = (high - low + 1) as u32;
    // Espalha a sequência (Knuth) antes de tomar o resto.
    low + (sequence.wrapping_mul(2_654_435_761) >> 16) as i32 % span as i32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobMode {
    /// Passeia pelo mapa.
    Patrol,
    /// Corre atrás do jogador.
    Chase,
    /// Bate no jogador.
    Attack,
    /// Reagindo a um golpe.
    Hurt,
    /// Caído até reaparecer.
    Dead,
}

/// Próximo modo de um monstro que está vivo e livre (`Patrol`, `Chase` ou `Attack`), pela distância
/// até o jogador. `Hurt` e `Dead` são resolvidos por tempo, fora daqui.
#[must_use]
pub fn mob_decision(mode: MobMode, distance: f32, player_alive: bool) -> MobMode {
    match mode {
        MobMode::Patrol => {
            if player_alive && distance <= AGGRO_RANGE {
                MobMode::Chase
            } else {
                MobMode::Patrol
            }
        }
        MobMode::Chase => {
            if !player_alive || distance > LEASH_RANGE {
                MobMode::Patrol
            } else if distance <= MOB_REACH {
                MobMode::Attack
            } else {
                MobMode::Chase
            }
        }
        MobMode::Attack => {
            if !player_alive || distance > LEASH_RANGE {
                MobMode::Patrol
            } else if distance > MOB_REACH * 1.3 {
                MobMode::Chase
            } else {
                MobMode::Attack
            }
        }
        other => other,
    }
}

/// Um raio (origem, direção unitária) acerta uma esfera? Usado para escolher o monstro com o clique.
#[must_use]
pub fn ray_hits_sphere(
    origin: [f32; 3],
    direction: [f32; 3],
    center: [f32; 3],
    radius: f32,
) -> bool {
    let offset = [
        center[0] - origin[0],
        center[1] - origin[1],
        center[2] - origin[2],
    ];
    let along = offset[0] * direction[0] + offset[1] * direction[1] + offset[2] * direction[2];
    if along < 0.0 {
        return false;
    }
    let squared = offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2];
    squared - along * along <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapon_type_and_motion_slots_follow_the_client() {
        // `m_byItemType = id / 200 + 1`; sem arma é 0.
        assert_eq!(item_type(0), 0);
        assert_eq!(item_type(1), 1, "Short Sword");
        assert_eq!(item_type(199), 1);
        assert_eq!(item_type(200), 2);
        // `SetAction(tipo * 50 + movimento)`; o slot do `.chr` é esse número menos 1.
        assert_eq!(player_slot(0, player_motion::WALK), 6);
        assert_eq!(
            player_slot(1, player_motion::ATTACK1_1),
            58,
            "pa01009 no pm01000.chr"
        );
        assert_eq!(player_slot(1, player_motion::STAND1), 53);
        assert_eq!(mob_slot(mob_motion::ATTACK1), 4);
        assert_eq!(mob_slot(mob_motion::DOWN), 14);
    }

    #[test]
    fn vitals_report_the_killing_blow_once() {
        let mut life = Vitals::full(20);
        assert!(!life.hurt(8));
        assert_eq!(life.fraction(), 0.6);
        assert!(life.hurt(50), "this one kills");
        assert_eq!(life.hp, 0);
        assert!(!life.alive());
        assert!(!life.hurt(5), "a fallen target is not killed twice");
        assert_eq!(life.fraction(), 0.0);
    }

    #[test]
    fn damage_rolls_stay_in_range_and_vary() {
        let rolls: Vec<i32> = (0..200).map(player_damage_roll).collect();
        assert!(
            rolls
                .iter()
                .all(|r| (PLAYER_DAMAGE.0..=PLAYER_DAMAGE.1).contains(r))
        );
        assert!(rolls.iter().any(|r| *r != rolls[0]), "not a constant");
    }

    #[test]
    fn the_mob_wakes_chases_attacks_and_gives_up_by_distance() {
        use MobMode::*;
        assert_eq!(mob_decision(Patrol, AGGRO_RANGE + 1.0, true), Patrol);
        assert_eq!(mob_decision(Patrol, AGGRO_RANGE - 0.5, true), Chase);
        assert_eq!(
            mob_decision(Patrol, 1.0, false),
            Patrol,
            "ignores a dead player"
        );
        assert_eq!(mob_decision(Chase, MOB_REACH - 0.1, true), Attack);
        assert_eq!(mob_decision(Chase, 3.0, true), Chase);
        assert_eq!(mob_decision(Chase, LEASH_RANGE + 1.0, true), Patrol);
        // Histerese: só volta a correr quando o jogador se afasta bem do alcance.
        assert_eq!(mob_decision(Attack, MOB_REACH * 1.2, true), Attack);
        assert_eq!(mob_decision(Attack, MOB_REACH * 1.5, true), Chase);
        assert_eq!(mob_decision(Attack, 1.0, false), Patrol);
        // Reagir e cair não mudam por distância.
        assert_eq!(mob_decision(Hurt, 0.5, true), Hurt);
        assert_eq!(mob_decision(Dead, 0.5, true), Dead);
    }

    #[test]
    fn rays_pick_a_sphere_only_in_front_of_them() {
        let origin = [0.0, 0.0, 0.0];
        let forward = [0.0, 0.0, -1.0];
        assert!(ray_hits_sphere(origin, forward, [0.0, 0.0, -5.0], 1.0));
        assert!(
            ray_hits_sphere(origin, forward, [0.8, 0.0, -5.0], 1.0),
            "grazing"
        );
        assert!(
            !ray_hits_sphere(origin, forward, [1.5, 0.0, -5.0], 1.0),
            "misses to the side"
        );
        assert!(
            !ray_hits_sphere(origin, forward, [0.0, 0.0, 5.0], 1.0),
            "behind the origin"
        );
    }
}
