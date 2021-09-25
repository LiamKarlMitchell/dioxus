use dioxus_core as dioxus;
use dioxus_core::prelude::*;
use dioxus_html as dioxus_elements;

fn main() {
    dioxus_desktop::launch(App, |f| f.with_window(|w| w.with_maximized(true))).expect("Failed");
}

static App: FC<()> = |cx, props|{
    //
    cx.render(rsx!(
        div {
            "hello world!"
        }
    ))
};