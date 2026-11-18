mod data_structures;
mod helpers;

use data_structures::App;

fn main() -> anyhow::Result<()> {
    helpers::prepend_local_bin_to_path();
    let mut terminal = ratatui::init();
    let result = App::new().run(&mut terminal);
    ratatui::restore();
    if let Some(command) = result? {
        println!("{command}");
    }
    Ok(())
}
