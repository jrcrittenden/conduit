use crate::ui::action::Action;
use crate::ui::app::App;
use crate::ui::events::InputMode;

impl App {
    pub(super) fn handle_coderabbit_feedback_action(&mut self, action: Action) {
        if self.state.input_mode != InputMode::CodeRabbitFeedback {
            return;
        }

        match action {
            Action::CodeRabbitToggleSelection => {
                self.state.coderabbit_feedback_state.toggle_selected();
            }
            Action::CodeRabbitSelectAll => {
                self.state.coderabbit_feedback_state.select_all_filtered();
            }
            Action::CodeRabbitSelectNone => {
                self.state.coderabbit_feedback_state.clear_selection();
            }
            Action::CodeRabbitCycleFilter => {
                self.state.coderabbit_feedback_state.cycle_filter();
            }
            _ => {}
        }
    }
}
