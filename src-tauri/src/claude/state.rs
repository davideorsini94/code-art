//! Macchina degli stati di una conversazione: funzione pura, nessun I/O.

use super::ConvStatus;
use super::protocol::ClaudeEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConvState {
    pub status: ConvStatus,
    pub open_asks: u32,
    pub turn_count: u32,
    pub seen_turn: u32,
}

#[derive(Debug, Clone, Copy)]
pub enum StateInput<'a> {
    UserSent,
    Event(&'a ClaudeEvent),
    /// L'utente ha risposto a una richiesta.
    AskAnswered,
    /// L'utente ha guardato la conversazione.
    Viewed,
    ProcessExited,
}

fn ask_closed(mut cur: ConvState) -> ConvState {
    if cur.open_asks == 0 {
        return cur;
    }
    cur.open_asks -= 1;
    if cur.open_asks == 0 {
        cur.status = ConvStatus::Working;
    }
    cur
}

pub fn next_status(cur: ConvState, input: StateInput) -> ConvState {
    let mut s = cur;
    match input {
        StateInput::UserSent => s.status = ConvStatus::Working,
        StateInput::AskAnswered => return ask_closed(cur),
        StateInput::Event(ClaudeEvent::AskCancelled { .. }) => return ask_closed(cur),
        StateInput::Event(ClaudeEvent::Ask { .. }) => {
            s.status = ConvStatus::Waiting;
            s.open_asks += 1;
        }
        StateInput::Event(ClaudeEvent::TurnEnd { .. }) => {
            // Un `result` non richiesto (subagent in background) a conversazione ferma non conta.
            if cur.status != ConvStatus::Working {
                return cur;
            }
            s.turn_count += 1;
            s.open_asks = 0;
            s.status = if s.seen_turn < s.turn_count { ConvStatus::Waiting } else { ConvStatus::Idle };
        }
        StateInput::Event(_) => {}
        StateInput::Viewed => {
            s.seen_turn = s.turn_count;
            if s.status == ConvStatus::Waiting && s.open_asks == 0 {
                s.status = ConvStatus::Idle;
            }
        }
        StateInput::ProcessExited => {
            if s.status == ConvStatus::Working {
                s.status = ConvStatus::Waiting;
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::protocol::AskType;

    fn st(status: ConvStatus, open_asks: u32, turn_count: u32, seen_turn: u32) -> ConvState {
        ConvState { status, open_asks, turn_count, seen_turn }
    }
    fn ask() -> ClaudeEvent {
        ClaudeEvent::Ask {
            request_id: "r".into(),
            ask_type: AskType::Permission,
            prompt: "p".into(),
            options: vec![],
            questions: vec![],
            tool_name: "Bash".into(),
            input: serde_json::json!({}),
        }
    }
    fn turn_end() -> ClaudeEvent {
        ClaudeEvent::TurnEnd { context_used_pct: None, is_error: false }
    }
    fn unknown() -> ClaudeEvent {
        ClaudeEvent::Unknown { label: "x".into(), json: "{}".into() }
    }
    fn run(start: ConvState, inputs: &[StateInput]) -> ConvState {
        inputs.iter().fold(start, |c, i| next_status(c, *i))
    }

    use ConvStatus::*;

    #[test]
    fn transition_table() {
        let a = ask();
        let te = turn_end();
        let cancelled = ClaudeEvent::AskCancelled { request_id: "r".into() };
        let cases: Vec<(&str, ConvState, StateInput, ConvState)> = vec![
            ("user sent from idle", st(Idle, 0, 0, 0), StateInput::UserSent, st(Working, 0, 0, 0)),
            ("user sent from waiting", st(Waiting, 0, 1, 0), StateInput::UserSent, st(Working, 0, 1, 0)),
            ("ask", st(Working, 0, 0, 0), StateInput::Event(&a), st(Waiting, 1, 0, 0)),
            ("second ask", st(Waiting, 1, 0, 0), StateInput::Event(&a), st(Waiting, 2, 0, 0)),
            ("answered, one left", st(Waiting, 2, 0, 0), StateInput::AskAnswered, st(Waiting, 1, 0, 0)),
            ("answered, none left", st(Waiting, 1, 0, 0), StateInput::AskAnswered, st(Working, 0, 0, 0)),
            ("answered with none open", st(Working, 0, 0, 0), StateInput::AskAnswered, st(Working, 0, 0, 0)),
            ("cancelled like answered", st(Waiting, 1, 0, 0), StateInput::Event(&cancelled), st(Working, 0, 0, 0)),
            ("turn end unseen", st(Working, 0, 0, 0), StateInput::Event(&te), st(Waiting, 0, 1, 0)),
            ("turn end already seen", st(Working, 0, 2, 3), StateInput::Event(&te), st(Idle, 0, 3, 3)),
            ("turn end while idle ignored", st(Idle, 0, 1, 1), StateInput::Event(&te), st(Idle, 0, 1, 1)),
            ("turn end while waiting ignored", st(Waiting, 1, 1, 0), StateInput::Event(&te), st(Waiting, 1, 1, 0)),
            ("viewed waiting no asks", st(Waiting, 0, 1, 0), StateInput::Viewed, st(Idle, 0, 1, 1)),
            ("viewed waiting with asks", st(Waiting, 1, 1, 0), StateInput::Viewed, st(Waiting, 1, 1, 1)),
            ("viewed working", st(Working, 0, 1, 0), StateInput::Viewed, st(Working, 0, 1, 1)),
            ("exit while working", st(Working, 0, 0, 0), StateInput::ProcessExited, st(Waiting, 0, 0, 0)),
            ("exit while idle", st(Idle, 0, 0, 0), StateInput::ProcessExited, st(Idle, 0, 0, 0)),
            ("exit while waiting", st(Waiting, 1, 0, 0), StateInput::ProcessExited, st(Waiting, 1, 0, 0)),
        ];
        for (name, from, input, want) in cases {
            assert_eq!(next_status(from, input), want, "{name}");
        }
    }

    #[test]
    fn unknown_event_changes_nothing() {
        let u = unknown();
        for s in [st(Working, 0, 0, 0), st(Waiting, 1, 2, 1), st(Idle, 0, 3, 3)] {
            assert_eq!(next_status(s, StateInput::Event(&u)), s);
        }
    }

    #[test]
    fn unknown_event_never_leaves_working_stuck() {
        let (u, te) = (unknown(), turn_end());
        let end = run(
            ConvState::default(),
            &[StateInput::UserSent, StateInput::Event(&u), StateInput::Event(&te)],
        );
        assert_eq!(end.status, Waiting);
        assert_eq!(end.turn_count, 1);
    }

    #[test]
    fn unrequested_turn_end_is_harmless() {
        let te = turn_end();
        let end = run(
            ConvState::default(),
            &[StateInput::UserSent, StateInput::Event(&te), StateInput::Viewed, StateInput::Event(&te)],
        );
        assert_eq!(end, st(Idle, 0, 1, 1));
    }

    #[test]
    fn full_permission_flow() {
        let (a, te) = (ask(), turn_end());
        let end = run(
            ConvState::default(),
            &[
                StateInput::UserSent,
                StateInput::Event(&a),
                StateInput::AskAnswered,
                StateInput::Event(&te),
                StateInput::Viewed,
            ],
        );
        assert_eq!(end, st(Idle, 0, 1, 1));
    }
}
