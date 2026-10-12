//! UI input events → IronRDP input operations.

use crate::proto::Input;
use ironrdp_input::{MouseButton, MousePosition, Operation, Scancode, WheelRotations};

/// The operations for one UI event; `None` for [`Input::ReleaseAll`], which the caller handles
/// with `Database::release_all`. Unknown buttons are dropped.
pub fn operations(input: &Input) -> Option<Vec<Operation>> {
    let button = |b: u8| MouseButton::from_web_button(b);
    Some(match *input {
        Input::Move { x, y } => vec![Operation::MouseMove(MousePosition { x, y })],
        Input::Down { button: b } => button(b)
            .map(Operation::MouseButtonPressed)
            .into_iter()
            .collect(),
        Input::Up { button: b } => button(b)
            .map(Operation::MouseButtonReleased)
            .into_iter()
            .collect(),
        Input::Wheel { vertical, units } => vec![Operation::WheelRotations(WheelRotations {
            is_vertical: vertical,
            rotation_units: units,
        })],
        Input::Key { code, down } => {
            let sc = Scancode::from_u16(code);
            vec![if down {
                Operation::KeyPressed(sc)
            } else {
                Operation::KeyReleased(sc)
            }]
        }
        Input::ReleaseAll => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_events() {
        assert!(matches!(
            operations(&Input::Down { button: 2 }).unwrap()[..],
            [Operation::MouseButtonPressed(MouseButton::Right)]
        ));
        assert!(operations(&Input::Down { button: 9 }).unwrap().is_empty());
        match &operations(&Input::Key {
            code: 0xE01D,
            down: false,
        })
        .unwrap()[..]
        {
            [Operation::KeyReleased(sc)] => assert_eq!(sc.as_u8(), (true, 0x1D)),
            other => panic!("{other:?}"),
        }
        assert!(operations(&Input::ReleaseAll).is_none());
    }
}
