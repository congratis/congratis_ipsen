use dominator::{append_dom, clone, events, html, Dom};
use futures_signals::signal::{Mutable, SignalExt};
use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{closure::Closure, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{EventSource, HtmlAudioElement, HtmlInputElement, HtmlTextAreaElement, MessageEvent};

#[derive(Clone, Deserialize, Serialize)]
struct User { subject_id: String, role: String, csrf_token: String }
#[derive(Clone, Deserialize, Serialize)]
struct Greeting { ornament_number: i16, display_name: String, message: String }
#[derive(Clone, Deserialize)]
struct Participant { ornament_number: i16, activated: bool, submitted: bool, revoked: bool, display_name: Option<String>, created_at: i64, used_at: Option<i64> }
#[derive(Clone, Deserialize)]
struct CampaignSummary { participant_count: usize, submitted_count: usize }
#[derive(Clone, Deserialize)]
struct CreatedInvitation { ornament_number: i16, token: String }
#[derive(Clone)]
struct InvitationLink { ornament_number: i16, url: String }

#[derive(Clone)]
struct AppState {
    user: Mutable<Option<User>>,
    greetings: Mutable<Vec<Greeting>>,
    participants: Mutable<Vec<Participant>>,
    campaign_count: Mutable<usize>,
    submitted_count: Mutable<usize>,
    selected: Mutable<Option<Greeting>>,
    presentation: Mutable<bool>,
    presentation_index: Mutable<usize>,
    presentation_ornament: Mutable<Option<i16>>,
    presentation_paused: Mutable<bool>,
    music_muted: Mutable<bool>,
    music_volume: Mutable<f64>,
    audio: Mutable<Option<HtmlAudioElement>>,
    name: Mutable<String>,
    message: Mutable<String>,
    admin_name: Mutable<String>,
    admin_password: Mutable<String>,
    note: Mutable<String>,
    invite_url: Mutable<String>,
    invite_urls: Mutable<Vec<InvitationLink>>,
    campaign_size: Mutable<String>,
    campaign_creating: Mutable<bool>,
    submitting: Mutable<bool>,
}
impl AppState {
    fn new() -> Self {
        Self { user: Mutable::new(None), greetings: Mutable::new(Vec::new()), participants: Mutable::new(Vec::new()), campaign_count: Mutable::new(0), submitted_count: Mutable::new(0), selected: Mutable::new(None), presentation: Mutable::new(false), presentation_index: Mutable::new(0), presentation_ornament: Mutable::new(None), presentation_paused: Mutable::new(false), music_muted: Mutable::new(false), music_volume: Mutable::new(0.65), audio: Mutable::new(None), name: Mutable::new(String::new()), message: Mutable::new(String::new()), admin_name: Mutable::new(String::new()), admin_password: Mutable::new(String::new()), note: Mutable::new(String::new()), invite_url: Mutable::new(String::new()), invite_urls: Mutable::new(Vec::new()), campaign_size: Mutable::new("20".into()), campaign_creating: Mutable::new(false), submitting: Mutable::new(false) }
    }
    fn csrf(&self) -> String { self.user.lock_ref().as_ref().map(|u| u.csrf_token.clone()).unwrap_or_default() }
}
async fn get_json<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, String> {
    let response = Request::get(path).send().await.map_err(|e| e.to_string())?;
    if !response.ok() { return Err(response.text().await.unwrap_or_else(|_| "Ошибка запроса".into())); }
    response.json::<T>().await.map_err(|e| e.to_string())
}
async fn post_json<T: for<'de> Deserialize<'de>>(path: &str, body: &impl Serialize, csrf: Option<&str>) -> Result<T, String> {
    let mut request = Request::post(path).header("content-type", "application/json");
    if let Some(token) = csrf { request = request.header("x-csrf-token", token); }
    let response = request.json(body).map_err(|e| e.to_string())?.send().await.map_err(|e| e.to_string())?;
    if !response.ok() { return Err(response.text().await.unwrap_or_else(|_| format!("HTTP {}", response.status()))); }
    response.json::<T>().await.map_err(|e| e.to_string())
}
async fn post_empty(path: &str, csrf: &str) -> Result<(), String> {
    let response = Request::post(path).header("x-csrf-token", csrf).send().await.map_err(|e| e.to_string())?;
    if response.ok() { Ok(()) } else { Err(response.text().await.unwrap_or_else(|_| format!("HTTP {}", response.status()))) }
}
fn set_note(state: &AppState, value: impl Into<String>) { state.note.set(value.into()); }
const JINGLE_BELLS_URL: &str = "https://upload.wikimedia.org/wikipedia/commons/f/f0/Jingle_Bells_%2890bpm%29_%28Kevin_MacLeod%29_%28ISRC_USUAN1100187%29.oga";

fn set_presentation_index(state: &AppState, index: usize) {
    let greetings = state.greetings.get_cloned();
    state.presentation_index.set(index);
    state.presentation_ornament.set(greetings.get(index).map(|greeting| greeting.ornament_number));
}

fn play_presentation_audio(state: AppState) {
    let audio = match state.audio.get_cloned() {
        Some(audio) => audio,
        None => match HtmlAudioElement::new_with_src(JINGLE_BELLS_URL) {
            Ok(audio) => {
                audio.set_loop(true);
                audio.set_volume(state.music_volume.get().clamp(0.0, 1.0));
                audio.set_muted(state.music_muted.get());
                state.audio.set(Some(audio.clone()));
                audio
            }
            Err(_) => {
                state.presentation_paused.set(true);
                set_note(&state, "Музыка недоступна — презентация продолжится без звука");
                return;
            }
        },
    };

    match audio.play() {
        Ok(promise) => {
            let current = state.clone();
            spawn_local(async move {
                if JsFuture::from(promise).await.is_err() {
                    if let Some(audio) = current.audio.get_cloned() {
                        audio.pause();
                    }
                    current.audio.set(None);
                    current.presentation_paused.set(true);
                    set_note(&current, "Браузер не разрешил музыку — презентация продолжится без звука");
                }
            });
        }
        Err(_) => {
            state.audio.set(None);
            state.presentation_paused.set(true);
            set_note(&state, "Музыка недоступна — презентация продолжится без звука");
        }
    }
}

fn start_presentation(state: AppState) {
    state.presentation.set(true);
    state.presentation_paused.set(false);
    set_presentation_index(&state, 0);
    if state.greetings.get_cloned().is_empty() {
        set_note(&state, "Пока нет опубликованных поздравлений");
    } else {
        play_presentation_audio(state);
    }
}

fn toggle_presentation_pause(state: AppState) {
    let paused = !state.presentation_paused.get();
    state.presentation_paused.set(paused);
    if paused {
        if let Some(audio) = state.audio.get_cloned() { audio.pause(); }
    } else {
        play_presentation_audio(state);
    }
}

fn toggle_music_mute(state: AppState) {
    let muted = !state.music_muted.get();
    state.music_muted.set(muted);
    if let Some(audio) = state.audio.get_cloned() { audio.set_muted(muted); }
}

fn set_music_volume(state: &AppState, volume: f64) {
    let volume = volume.clamp(0.0, 1.0);
    state.music_volume.set(volume);
    if let Some(audio) = state.audio.get_cloned() { audio.set_volume(volume); }
}

fn exit_presentation(state: AppState) {
    if let Some(audio) = state.audio.get_cloned() { audio.pause(); }
    state.audio.set(None);
    state.presentation.set(false);
    state.presentation_paused.set(false);
    state.presentation_ornament.set(None);
}

async fn load_greetings(state: AppState) {
    match get_json::<Vec<Greeting>>("/api/greetings").await {
        Ok(mut items) => {
            // Preserve committed SSE updates that may arrive while this snapshot is in flight.
            for greeting in state.greetings.get_cloned() {
                if !items.iter().any(|item| item.ornament_number == greeting.ornament_number) {
                    items.push(greeting);
                }
            }
            items.sort_by_key(|greeting| greeting.ornament_number);
            state.submitted_count.set(items.len());
            state.greetings.set(items);
        },
        Err(error) => set_note(&state, format!("Не удалось загрузить поздравления: {error}")),
    }
}
async fn load_participants(state: AppState) {
    match get_json::<Vec<Participant>>("/api/admin/participants").await {
        Ok(items) => state.participants.set(items),
        Err(error) => set_note(&state, format!("Не удалось загрузить участников: {error}")),
    }
}
async fn load_campaign(state: AppState) {
    match get_json::<CampaignSummary>("/api/campaign").await {
        Ok(summary) => {
            state.campaign_count.set(summary.participant_count);
            state.submitted_count.set(summary.submitted_count);
        },
        Err(error) => set_note(&state, format!("Не удалось загрузить параметры кампании: {error}")),
    }
}
fn ornament_positions(count: usize) -> Vec<(f64, f64)> {
    const DEFAULT: [(f64, f64); 20] = [
        (50.2,18.3),(46.2,27.9),(55.1,28.7),(41.5,38.1),(50.5,38.5),
        (59.1,38.6),(37.7,49.8),(46.5,50.3),(55.2,50.5),(63.8,50.7),
        (35.9,60.4),(45.0,61.0),(55.3,61.1),(64.5,61.6),(32.3,72.5),
        (42.7,73.2),(56.0,73.6),(66.0,73.3),(45.4,85.2),(59.7,85.5)
    ];
    if count == 20 { return DEFAULT.to_vec(); }
    let row_capacity = [1usize, 2, 3, 4, 5, 6, 7, 7];
    let row_y = [17.5, 27.0, 36.5, 46.0, 55.5, 65.0, 74.5, 84.0];
    let mut remaining = count.min(35);
    let mut points = Vec::with_capacity(remaining);
    for (row, capacity) in row_capacity.iter().enumerate() {
        if remaining == 0 { break; }
        let columns = remaining.min(*capacity);
        let half_span = if columns <= 1 { 0.0 } else { 5.0 + row as f64 * 1.9 };
        for column in 0..columns {
            let offset = if columns <= 1 {
                0.0
            } else {
                (column as f64 / (columns - 1) as f64) * 2.0 - 1.0
            };
            points.push((50.0 + offset * half_span, row_y[row]));
        }
        remaining -= columns;
    }
    points
}
fn connect_events(state: AppState) {
    let Ok(source) = EventSource::new("/api/events") else { return; };
    let greeting_state = state.clone();
    let greeting_handler = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        if let Ok(message) = event.dyn_into::<MessageEvent>() {
            if let Some(raw) = message.data().as_string() {
                if let Ok(greeting) = serde_json::from_str::<Greeting>(&raw) {
                    let mut all = greeting_state.greetings.lock_mut();
                    if let Some(old) = all.iter_mut().find(|g| g.ornament_number == greeting.ornament_number) { *old = greeting; }
                    else { all.push(greeting); }
                    all.sort_by_key(|item| item.ornament_number);
                    let submitted_count = all.len();
                    drop(all);
                    greeting_state.submitted_count.set(submitted_count);
                }
            }
        }
    });
    let _ = source.add_event_listener_with_callback("greeting", greeting_handler.as_ref().unchecked_ref());
    greeting_handler.forget();
    let resync_state = state.clone();
    let resync_handler = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        let state = resync_state.clone();
        spawn_local(async move {
            load_greetings(state.clone()).await;
            load_campaign(state).await;
        });
    });
    let _ = source.add_event_listener_with_callback("resync", resync_handler.as_ref().unchecked_ref());
    resync_handler.forget();
    // EventSource fires "open" both on the initial connection and after reconnects.
    // Reconcile against the API so a missed SSE notification cannot leave stale progress.
    let open_state = state.clone();
    let open_handler = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        let state = open_state.clone();
        spawn_local(async move {
            load_greetings(state.clone()).await;
            load_campaign(state).await;
        });
    });
    let _ = source.add_event_listener_with_callback("open", open_handler.as_ref().unchecked_ref());
    open_handler.forget();
    std::mem::forget(source);
}
fn field(label: &'static str, placeholder: &'static str, value: Mutable<String>, is_textarea: bool) -> Dom {
    if is_textarea {
        html!("label", { .text(label).child(html!("textarea", {
            .attr("rows", "4").attr("maxlength", "500").attr("placeholder", placeholder)
            .property("value", &value.get_cloned())
            .event(clone!(value => move |event: events::Input| {
                if let Some(target) = event.target() { if let Ok(input) = target.dyn_into::<HtmlTextAreaElement>() { value.set(input.value()); } }
            }))
        })) })
    } else {
        html!("label", { .text(label).child(html!("input", {
            .attr("type", "text").attr("maxlength", "80").attr("placeholder", placeholder)
            .property("value", &value.get_cloned())
            .event(clone!(value => move |event: events::Input| {
                if let Some(target) = event.target() { if let Ok(input) = target.dyn_into::<HtmlInputElement>() { value.set(input.value()); } }
            }))
        })) })
    }
}
fn participant_tree(state: AppState) -> Dom {
    let user = state.user.lock_ref().clone().unwrap();
    let is_mine = user.role == "participant";
    let pill_label = if is_mine { format!("Твой шар №{}", user.subject_id) } else { "Режим администратора".to_string() };
    html!("main", {
        .class("shell")
        .child(html!("div", {.class("eyebrow").text("A LITTLE CHRISTMAS MAGIC")}))
        .child(html!("h1", {.text("Ёлка добрых ").child(html!("span", {.text("пожеланий")}))}))
        .child(html!("p", {.class("lead").text("Выбери шар — внутри тебя ждёт новогоднее поздравление. Все пожелания команды собраны на одной ёлке.")}))
        .child(html!("div", {
            .class("toolbar")
            .child(html!("span", {
                .class("pill")
                .text(&pill_label)
            }))
            .child(html!("button", {
                .class("secondary")
                .text("Админка")
                .event(clone!(state => move |_: events::Click| {
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().set_hash("/admin");
                        let _ = window.location().reload();
                    }
                }))
            }))
            .child(html!("button", {
                .class("secondary")
                .text("Выйти")
                .event(clone!(state => move |_: events::Click| {
                    let csrf = state.csrf();
                    let s = state.clone();
                    spawn_local(async move {
                        match post_empty("/api/auth/logout", &csrf).await {
                            Ok(()) => {
                                s.user.set(None);
                                if let Some(window) = web_sys::window() {
                                    let _ = window.location().set_hash("");
                                    let _ = window.location().reload();
                                }
                            }
                            Err(error) => set_note(&s, error),
                        }
                    });
                }))
            }))
        }))
        .child(html!("div", {.class("tree-frame")
            .child(html!("img", {.class("tree-art").attr("src","/christmas-tree.webp").attr("alt","Иллюстрированная новогодняя ёлка")}))
            .child_signal(state.greetings.signal_cloned().map(clone!(state => move |greeting_snapshot| {
                let points = ornament_positions(state.campaign_count.get());
                Some(html!("div", {.class("hotspots").children(points.iter().enumerate().map(|(index,(x,y))| {
                    let number=(index+1) as i16;
                    let greeting=greeting_snapshot.iter().find(|g|g.ornament_number==number).cloned();
                    let mine=is_mine && user.subject_id.parse::<i16>().ok()==Some(number);
                    html!("button", {
                        .class("ornament")
                        .class_signal("dense", futures_signals::signal::always(points.len() > 20))
                        .class_signal("filled", futures_signals::signal::always(greeting.is_some()))
                        .class_signal("mine", futures_signals::signal::always(mine))
                        .class_signal("presenting", state.presentation_ornament.signal_cloned().map(move |selected| selected == Some(number)))
                        .style("left", &format!("{x}%")).style("top",&format!("{y}%"))
                        .attr("aria-label",&format!("Шар номер {number}"))
                        .text(&number.to_string())
                        .event(clone!(state, greeting => move |_: events::Click| {
                            if let Some(g)=greeting.clone() { state.selected.set(Some(g)); }
                            else if mine { show_form(state.clone()); }
                            else { set_note(&state, "Этот шар пока свободен"); }
                        }))
                    })
                }))}))
            })))
            .child_signal(state.campaign_count.signal().map(clone!(state => move |participant_count| {
                Some(html!("div", {
                    .class("tree-progress")
                    .child_signal({
                        let progress_state = state.clone();
                        state.submitted_count.signal().map(move |submitted_count| {
                        let button_state = progress_state.clone();
                        let safe_submitted = submitted_count.min(participant_count);
                        let percent = if participant_count == 0 { 0.0 } else {
                            safe_submitted as f64 / participant_count as f64 * 100.0
                        };
                        Some(html!("div", {
                            .class("progress-content")
                            .child(html!("div", {
                                .class("candy-track")
                                .attr("role", "progressbar")
                                .attr("aria-label", "Опубликованные поздравления")
                                .attr("aria-valuemin", "0")
                                .attr("aria-valuemax", &participant_count.to_string())
                                .attr("aria-valuenow", &safe_submitted.to_string())
                                .child(html!("div", {
                                    .class("candy-fill")
                                    .style("width", &format!("{percent:.2}%"))
                                }))
                            }))
                            .child(html!("span", {
                                .class("progress-count")
                                .text(&format!("{safe_submitted} / {participant_count}"))
                            }))
                            .child(html!("button", {
                                .class("presentation-launch")
                                .attr("type", "button")
                                .text("▶ Play")
                                 .event(clone!(button_state => move |_: events::Click| start_presentation(button_state.clone())))
                            }))
                        }))
                    })
                    })
                }))
            })))
        }))
        .child(html!("div", {.class("legend")
            .child(html!("span", {.child(html!("i", {.class("dot")})).text("Свободный шар")}))
            .child(html!("span", {.child(html!("i", {.class("dot").class("filled")})).text("Опубликованное поздравление")}))
        }))
        .child(html!("div", {.class("note").text_signal(state.note.signal_cloned())}))
        .child(html!("footer", {.text("С теплом и верой в маленькие чудеса ✨")}))
        .child_signal(state.selected.signal_cloned().map(clone!(state => move |selected| {
            selected.map(|greeting| {
                let ordered = state.greetings.get_cloned();
                let current_index = ordered.iter().position(|item| item.ornament_number == greeting.ornament_number);
                let previous = current_index.and_then(|index| index.checked_sub(1)).and_then(|index| ordered.get(index)).cloned();
                let next = current_index.and_then(|index| ordered.get(index + 1)).cloned();
                let counter = current_index.map(|index| format!("Поздравление {} из {}", index + 1, ordered.len())).unwrap_or_default();
                html!("div", {
                    .class("overlay")
                    .child(html!("section", {
                        .class("dialog")
                        .attr("role", "dialog")
                        .attr("aria-modal", "true")
                        .attr("aria-labelledby", "greeting-title")
                        .child(html!("button", {
                            .class("close")
                            .attr("type", "button")
                            .attr("aria-label", "Закрыть поздравление")
                            .text("×")
                            .event(clone!(state => move |_: events::Click| state.selected.set(None)))
                        }))
                        .child(html!("div", {
                            .class("kicker")
                            .text("CHRISTMAS WISH")
                        }))
                        .child(html!("h2", {
                            .attr("id", "greeting-title")
                            .text(&format!("С теплом, {}", greeting.display_name))
                        }))
                        .child(html!("p", {
                            .class("greeting-message")
                            .text(&greeting.message)
                        }))
                        .child(html!("div", {
                            .class("greeting-navigation")
                            .child(match previous.clone() {
                                Some(item) => html!("button", {
                                    .class("secondary")
                                    .attr("type", "button")
                                    .attr("aria-label", "Предыдущее поздравление")
                                    .text("← Назад")
                                    .event(clone!(state => move |_: events::Click| state.selected.set(Some(item.clone()))))
                                }),
                                None => html!("button", {
                                    .class("secondary")
                                    .attr("type", "button")
                                    .attr("disabled", "")
                                    .attr("aria-label", "Предыдущее поздравление")
                                    .text("← Назад")
                                })
                            })
                            .child(html!("span", {
                                .class("greeting-counter")
                                .text(&counter)
                            }))
                            .child(match next.clone() {
                                Some(item) => html!("button", {
                                    .class("secondary")
                                    .attr("type", "button")
                                    .attr("aria-label", "Следующее поздравление")
                                    .text("Вперёд →")
                                    .event(clone!(state => move |_: events::Click| state.selected.set(Some(item.clone()))))
                                }),
                                None => html!("button", {
                                    .class("secondary")
                                    .attr("type", "button")
                                    .attr("disabled", "")
                                    .attr("aria-label", "Следующее поздравление")
                                    .text("Вперёд →")
                                })
                            })
                        }))
                        .child(html!("button", {
                            .class("primary")
                            .attr("type", "button")
                            .text("Вернуться к ёлке")
                            .event(clone!(state => move |_: events::Click| state.selected.set(None)))
                        }))
                    }))
                })
            })
        })))
        .child_signal(state.presentation.signal().map(clone!(state => move |active| {
            if !active {
                None
            } else {
                Some(html!("div", {
                    .class("presentation-overlay")
                    .child(html!("section", {
                        .class("presentation-card")
                        .attr("role", "dialog")
                        .attr("aria-modal", "true")
                        .attr("aria-labelledby", "presentation-title")
                        .child_signal(state.presentation_index.signal().map(clone!(state => move |index| {
                            let greetings = state.greetings.get_cloned();
                            let count = greetings.len();
                            let current = greetings.get(index).cloned();
                            let (title, message, counter) = if let Some(greeting) = current {
                                (
                                    format!("С теплом, {}", greeting.display_name),
                                    greeting.message,
                                    format!("Поздравление {} из {}", index + 1, count)
                                )
                            } else if count == 0 {
                                (
                                    "Ёлка ждёт добрых слов".to_string(),
                                    "Пока никто не добавил поздравление. Вернись сюда, когда появятся первые пожелания.".to_string(),
                                    "Пока пусто".to_string()
                                )
                            } else {
                                (
                                    "С Новым годом!".to_string(),
                                    "Спасибо, что поделились теплом и сделали этот год чуточку светлее.".to_string(),
                                    format!("Все {} поздравлений просмотрены", count)
                                )
                            };
                            Some(html!("div", {
                                .class("presentation-content")
                                .class(if index == count && count > 0 { "presentation-finale" } else { "presentation-slide" })
                                .child(html!("div", {.class("kicker").text("CONGRATIS · NEW YEAR PRESENTATION")}))
                                .child(html!("h2", {
                                    .attr("id", "presentation-title")
                                    .text(&title)
                                }))
                                .child(html!("p", {
                                    .class("presentation-message")
                                    .text(&message)
                                }))
                                .child(html!("div", {
                                    .class("presentation-counter")
                                    .text(&counter)
                                }))
                            }))
                        })))
                        .child(html!("div", {
                            .class("presentation-controls")
                            .child(html!("button", {
                                .class("secondary")
                                .attr("type", "button")
                                .attr("aria-label", "Предыдущее поздравление")
                                .text("← Назад")
                                .event(clone!(state => move |_: events::Click| {
                                    let current = state.presentation_index.get();
                                    if current > 0 { set_presentation_index(&state, current - 1); }
                                }))
                            }))
                            .child(html!("button", {
                                .class("secondary")
                                .attr("type", "button")
                                .attr("aria-label", "Следующее поздравление")
                                .text("Вперёд →")
                                .event(clone!(state => move |_: events::Click| {
                                    let current = state.presentation_index.get();
                                    let total = state.greetings.lock_ref().len();
                                    if current < total { set_presentation_index(&state, current + 1); }
                                }))
                            }))
                        }))
                        .child(html!("div", {
                            .class("presentation-audio-controls")
                            .child(html!("button", {
                                .class("secondary")
                                .attr("type", "button")
                                .attr("aria-label", "Пауза или продолжение музыки")
                                .text_signal(state.presentation_paused.signal().map(|paused| if paused { "Продолжить музыку" } else { "Пауза музыки" }))
                                .event(clone!(state => move |_: events::Click| toggle_presentation_pause(state.clone())))
                            }))
                            .child(html!("button", {
                                .class("secondary")
                                .attr("type", "button")
                                .attr("aria-label", "Включить или выключить звук")
                                .text_signal(state.music_muted.signal().map(|muted| if muted { "Включить звук" } else { "Выключить звук" }))
                                .event(clone!(state => move |_: events::Click| toggle_music_mute(state.clone())))
                            }))
                            .child(html!("label", {
                                .class("volume-control")
                                .text("Громкость")
                                .child(html!("input", {
                                    .attr("type", "range")
                                    .attr("min", "0")
                                    .attr("max", "1")
                                    .attr("step", "0.05")
                                    .attr("aria-label", "Громкость музыки")
                                    .property("value", &state.music_volume.get().to_string())
                                    .event(clone!(state => move |event: events::Input| {
                                        if let Some(target) = event.target() {
                                            if let Ok(input) = target.dyn_into::<HtmlInputElement>() {
                                                set_music_volume(&state, input.value_as_number());
                                            }
                                        }
                                    }))
                                }))
                            }))
                        }))
                        .child(html!("div", {
                            .class("presentation-footer-controls")
                            .child(html!("button", {
                                .class("secondary")
                                .attr("type", "button")
                                .text("Повторить")
                                .event(clone!(state => move |_: events::Click| {
                                    set_presentation_index(&state, 0);
                                    state.presentation_paused.set(false);
                                    play_presentation_audio(state.clone());
                                }))
                            }))
                            .child(html!("button", {
                                .class("primary")
                                .attr("type", "button")
                                .text("Вернуться к ёлке")
                                .event(clone!(state => move |_: events::Click| exit_presentation(state.clone())))
                            }))
                        }))
                        .child(html!("p", {
                            .class("audio-attribution")
                            .text("Музыка: ")
                            .child(html!("a", {
                                .attr("href", "https://commons.wikimedia.org/wiki/File:Jingle_Bells.ogg")
                                .attr("target", "_blank")
                                .attr("rel", "noopener noreferrer")
                                .text("Jingle Bells — Kevin MacLeod")
                            }))
                            .child(html!("span", {.text(" · CC BY 3.0")}))
                        }))
                    }))
                }))
            }
        })))
    })
}
fn show_form(state: AppState) {
    let form_state = state.clone();
    let dialog = html!("div", {
        .class("overlay")
        .child(html!("section", {
            .class("dialog")
            .child(html!("button", {
                .class("close")
                .text("×")
                .event(|_: events::Click| {
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().reload();
                    }
                })
            }))
            .child(html!("div", {
                .class("kicker")
                .text("YOUR PERSONAL ORNAMENT")
            }))
            .child(html!("h2", {
                .text("Добавь своё пожелание")
            }))
            .child(html!("p", {
                .class("muted")
                .text("Твоё приглашение одноразовое. После публикации поздравление появится у всей команды.")
            }))
            .child(field("Как тебя зовут?", "Твоё имя", state.name.clone(), false))
            .child(field("Твоё поздравление", "Пусть в новом году...", state.message.clone(), true))
            .child(html!("div", {
                .class("note")
                .text_signal(state.note.signal_cloned())
            }))
            .child(html!("button", {
                .class("primary")
                .text("Опубликовать поздравление")
                .event(clone!(form_state => move |_: events::Click| {
                    if form_state.submitting.get() {
                        return;
                    }
                    let display_name = form_state.name.get_cloned();
                    if display_name.trim().is_empty() || display_name.chars().count() > 80 {
                        set_note(&form_state, "Имя должно содержать от 1 до 80 символов");
                        return;
                    }
                    let message = form_state.message.get_cloned();
                    if message.trim().is_empty() || message.chars().count() > 500 {
                        set_note(&form_state, "Поздравление должно содержать от 1 до 500 символов");
                        return;
                    }
                    form_state.submitting.set(true);
                    let csrf = form_state.csrf();
                    let current = form_state.clone();
                    spawn_local(async move {
                        let body = serde_json::json!({
                            "display_name": display_name,
                            "message": message
                        });
                        match post_json::<serde_json::Value>("/api/greetings", &body, Some(&csrf)).await {
                            Ok(_) => {
                                set_note(&current, "Поздравление опубликовано! ✨");
                                current.name.set(String::new());
                                current.message.set(String::new());
                                load_greetings(current.clone()).await;
                                if let Some(window) = web_sys::window() {
                                    let _ = window.location().reload();
                                }
                            }
                            Err(error) => {
                                current.submitting.set(false);
                                set_note(&current, error);
                            }
                        }
                    });
                }))
            }))
        }))
    });
    if let Some(document) = web_sys::window().and_then(|window| window.document()) {
        if let Some(root) = document.get_element_by_id("app") {
            append_dom(&root, dialog);
        }
    }
}

fn admin_screen(state: AppState) -> Dom {
    html!("main", {
        .class("shell")
        .child(html!("div", {
            .class("eyebrow")
            .text("CONGRATIS · CONTROL ROOM")
        }))
        .child(html!("h1", {
            .text("Панель ")
            .child(html!("span", {.text("администратора")}))
        }))
        .child(html!("p", {
            .class("lead")
            .text("Управление приглашениями и статусом участников кампании.")
        }))
        .child(html!("div", {
            .class("toolbar")
            .child(html!("button", {
                .class("secondary")
                .text("К ёлке")
                .event(|_: events::Click| {
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().set_hash("");
                        let _ = window.location().reload();
                    }
                })
            }))
            .child(html!("button", {
                .class("secondary")
                .text("Обновить")
                .event(clone!(state => move |_: events::Click| {
                    let current = state.clone();
                    spawn_local(async move {
                        load_participants(current).await;
                    });
                }))
            }))
        }))
        .child_signal(state.participants.signal_cloned().map(clone!(state => move |participants| {
            if participants.is_empty() {
                Some(html!("section", {
                    .class("panel")
                    .class("admin-create-campaign")
                    .child(html!("div", {.class("kicker").text("NEW CAMPAIGN")}))
                    .child(html!("h2", {.text("Создать новогоднюю кампанию")}))
                    .child(html!("p", {
                        .class("muted")
                        .text("Выбери количество участников. Для каждого будет создана отдельная одноразовая ссылка.")
                    }))
                    .child(html!("label", {
                        .text("Количество участников (от 1 до 35)")
                        .child(html!("input", {
                            .attr("type", "number")
                            .attr("min", "1")
                            .attr("max", "35")
                            .attr("step", "1")
                            .attr("aria-label", "Количество участников")
                            .property("value", &state.campaign_size.get_cloned())
                            .event(clone!(state => move |event: events::Input| {
                                if let Some(target) = event.target() {
                                    if let Ok(input) = target.dyn_into::<HtmlInputElement>() {
                                        state.campaign_size.set(input.value());
                                    }
                                }
                            }))
                        }))
                    }))
                    .child(html!("button", {
                        .class("primary")
                        .text_signal(state.campaign_creating.signal().map(|creating| if creating { "Создаём кампанию…" } else { "Создать кампанию и ссылки" }))
                        .event(clone!(state => move |_: events::Click| {
                            if state.campaign_creating.get() { return; }
                            let count = state.campaign_size.get_cloned().parse::<i16>();
                            let count = match count {
                                Ok(count) if (1..=35).contains(&count) => count,
                                _ => {
                                    set_note(&state, "Количество участников должно быть от 1 до 35");
                                    return;
                                }
                            };
                            state.campaign_creating.set(true);
                            let csrf = state.csrf();
                            let current = state.clone();
                            spawn_local(async move {
                                let body = serde_json::json!({ "participant_count": count });
                                match post_json::<Vec<CreatedInvitation>>("/api/admin/campaign", &body, Some(&csrf)).await {
                                    Ok(created) => {
                                        let origin = web_sys::window()
                                            .map(|window| window.location().origin().unwrap_or_default())
                                            .unwrap_or_default();
                                        let links = created.into_iter().map(|invitation| InvitationLink {
                                            ornament_number: invitation.ornament_number,
                                            url: format!("{origin}/#invite={}", invitation.token),
                                        }).collect();
                                        current.invite_urls.set(links);
                                        load_participants(current.clone()).await;
                                        load_campaign(current.clone()).await;
                                        set_note(&current, "Кампания создана. Сохраните одноразовые ссылки — повторно получить их будет нельзя.");
                                    }
                                    Err(error) => set_note(&current, error),
                                }
                                current.campaign_creating.set(false);
                            });
                        }))
                    }))
                }))
            } else {
                Some(html!("div", {
                    .class("admin-grid")
                    .children(participants.into_iter().map(clone!(state => move |participant| {
                        let number = participant.ornament_number;
                        let display_name = participant.display_name.unwrap_or_default();
                        let status = if participant.submitted {
                            "Поздравление опубликовано"
                        } else if participant.activated {
                            "Приглашение активировано"
                        } else if participant.revoked {
                            "Приглашение отозвано"
                        } else {
                            "Ожидает участника"
                        };
                        html!("div", {
                            .class("admin-item")
                            .child(html!("strong", {.text(&format!("Шар №{number}"))}))
                            .child(html!("small", {.text(status)}))
                            .child(html!("small", {.text(&display_name)}))
                            .children(if !participant.activated && !participant.submitted && !participant.revoked {
                                vec![
                                    html!("button", {
                                        .class("secondary")
                                        .text("Перевыпустить ссылку")
                                        .event(clone!(state => move |_: events::Click| {
                                            let csrf = state.csrf();
                                            let current = state.clone();
                                            spawn_local(async move {
                                                let path = format!("/api/admin/invitations/{number}/reissue");
                                                let body = serde_json::json!({});
                                                match post_json::<serde_json::Value>(&path, &body, Some(&csrf)).await {
                                                    Ok(data) => {
                                                        let token = data["token"].as_str().unwrap_or_default();
                                                        let origin = web_sys::window()
                                                            .map(|window| window.location().origin().unwrap_or_default())
                                                            .unwrap_or_default();
                                                        let url = format!("{origin}/#invite={token}");
                                                        current.invite_url.set(url.clone());
                                                        set_note(&current, format!("Одноразовая ссылка для шара №{number}: {url}"));
                                                        load_participants(current.clone()).await;
                                                    }
                                                    Err(error) => set_note(&current, error),
                                                }
                                            });
                                        }))
                                    }),
                                    html!("button", {
                                        .class("secondary")
                                        .text("Отозвать")
                                        .event(clone!(state => move |_: events::Click| {
                                            let csrf = state.csrf();
                                            let current = state.clone();
                                            spawn_local(async move {
                                                let path = format!("/api/admin/invitations/{number}/revoke");
                                                match post_empty(&path, &csrf).await {
                                                    Ok(()) => load_participants(current).await,
                                                    Err(error) => set_note(&current, error),
                                                }
                                            });
                                        }))
                                    })
                                ]
                            } else { vec![] })
                        })
                    })))
                }))
            }
        })))
        .child_signal(state.invite_urls.signal_cloned().map(|links| {
            if links.is_empty() {
                None
            } else {
                Some(html!("section", {
                    .class("admin-invite-list")
                    .child(html!("h2", {.text("Одноразовые ссылки кампании")}))
                    .child(html!("p", {
                        .class("muted")
                        .text("Скопируй и сохрани каждую ссылку сейчас. После закрытия страницы восстановить токены нельзя.")
                    }))
                    .children(links.into_iter().map(|invite| {
                        html!("label", {
                            .text(&format!("Шар №{}", invite.ornament_number))
                            .child(html!("input", {
                                .attr("type", "url")
                                .attr("readonly", "")
                                .attr("aria-label", &format!("Ссылка для шара №{}", invite.ornament_number))
                                .property("value", &invite.url)
                                .event(|event: events::Click| {
                                    if let Some(target) = event.target() {
                                        if let Ok(input) = target.dyn_into::<HtmlInputElement>() { input.select(); }
                                    }
                                })
                            }))
                        })
                    }))
                }))
            }
        }))
        .child_signal(state.invite_url.signal_cloned().map(|url| {
            if url.is_empty() {
                None
            } else {
                Some(html!("div", {
                    .class("admin-invite")
                    .child(html!("p", {
                        .text("Одноразовая ссылка. Нажми на поле, чтобы выделить её, затем скопируй.")
                    }))
                    .child(html!("input", {
                        .attr("type", "url")
                        .attr("readonly", "")
                        .attr("aria-label", "Одноразовая ссылка-приглашение")
                        .property("value", &url)
                        .event(|event: events::Click| {
                            if let Some(target) = event.target() {
                                if let Ok(input) = target.dyn_into::<HtmlInputElement>() {
                                    input.select();
                                }
                            }
                        })
                    }))
                }))
            }
        }))
        .child(html!("div", {
            .class("note")
            .text_signal(state.note.signal_cloned())
        }))
        .child(html!("footer", {
            .text("Ссылки-приглашения показываются только один раз. Сохраните их сразу после выпуска.")
        }))
    })
}
fn admin_login(state: AppState) -> Dom {
    html!("main", {
        .class("shell")
        .child(html!("div", {
            .class("eyebrow")
            .text("CONGRATIS · ADMIN")
        }))
        .child(html!("h1", {
            .text("Вход в ")
            .child(html!("span", {.text("админку")}))
        }))
        .child(html!("section", {
            .class("panel")
            .child(html!("div", {
                .class("kicker")
                .text("AUTHORIZED ACCESS")
            }))
            .child(field("Логин", "admin", state.admin_name.clone(), false))
            .child(html!("label", {
                .text("Пароль")
                .child(html!("input", {
                    .attr("type", "password")
                    .attr("placeholder", "Пароль администратора")
                    .event(clone!(state => move |event: events::Input| {
                        if let Some(target) = event.target() {
                            if let Ok(input) = target.dyn_into::<HtmlInputElement>() {
                                state.admin_password.set(input.value());
                            }
                        }
                    }))
                }))
            }))
            .child(html!("div", {
                .class("note")
                .text_signal(state.note.signal_cloned())
            }))
            .child(html!("button", {
                .class("primary")
                .text("Войти")
                .event(clone!(state => move |_: events::Click| {
                    let username = state.admin_name.get_cloned();
                    let password = state.admin_password.get_cloned();
                    let current = state.clone();
                    spawn_local(async move {
                        let body = serde_json::json!({
                            "username": username,
                            "password": password
                        });
                        match post_json::<User>("/api/auth/admin/login", &body, None).await {
                            Ok(user) => {
                                if let Some(window) = web_sys::window() {
                                    let _ = window.location().set_hash("/admin");
                                }
                                current.user.set(Some(user));
                                load_participants(current.clone()).await;
                            }
                            Err(error) => set_note(&current, error),
                        }
                    });
                }))
            }))
        }))
        .child(html!("footer", {
            .text("Доступ только для администратора.")
        }))
    })
}
fn landing_screen() -> Dom {
    html!("main", {
        .class("shell")
        .child(html!("div", {
            .class("eyebrow")
            .text("A LITTLE CHRISTMAS MAGIC")
        }))
        .child(html!("h1", {
            .text("Ёлка добрых ")
            .child(html!("span", {.text("пожеланий")}))
        }))
        .child(html!("p", {
            .class("lead")
            .text("Чтобы увидеть общую ёлку, открой персональную ссылку-приглашение.")
        }))
        .child(html!("button", {
            .class("primary")
            .text("Вход администратора")
            .event(|_: events::Click| {
                if let Some(window) = web_sys::window() {
                    let _ = window.location().set_hash("/admin");
                    let _ = window.location().reload();
                }
            })
        }))
    })
}

fn render(state: AppState) -> Dom {
    html!("div", {
        .child_signal(state.user.signal_cloned().map(clone!(state => move |user| {
            Some(if let Some(user) = user {
                if user.role == "admin" {
                    let is_admin = web_sys::window()
                        .map(|window| window.location().hash().unwrap_or_default().contains("/admin"))
                        .unwrap_or(false);
                    if is_admin {
                        admin_screen(state.clone())
                    } else {
                        participant_tree(state.clone())
                    }
                } else {
                    participant_tree(state.clone())
                }
            } else {
                let hash = web_sys::window()
                    .map(|window| window.location().hash().unwrap_or_default())
                    .unwrap_or_default();
                if hash.starts_with("#invite=") {
                    let token = hash.trim_start_matches("#invite=").to_string();
                    let current = state.clone();
                    spawn_local(async move {
                        let body = serde_json::json!({"token": token});
                        match post_json::<User>(
                            "/api/auth/participant/exchange",
                            &body,
                            None,
                        ).await {
                            Ok(user) => {
                                if let Some(window) = web_sys::window() {
                                    if let Ok(history) = window.history() {
                                        let _ = history.replace_state_with_url(
                                            &wasm_bindgen::JsValue::NULL,
                                            "",
                                            Some("/"),
                                        );
                                    }
                                }
                                current.user.set(Some(user));
                                load_campaign(current.clone()).await;
                                load_greetings(current.clone()).await;
                                connect_events(current);
                            }
                            Err(error) => {
                                set_note(&current, format!("Не удалось активировать приглашение: {error}"));
                            }
                        }
                    });
                    html!("main", {
                        .class("shell")
                        .child(html!("h1", {.text("Открываем твоё приглашение…")}))
                        .child(html!("p", {
                            .class("note")
                            .text_signal(state.note.signal_cloned())
                        }))
                    })
                } else if hash.contains("/admin") {
                    admin_login(state.clone())
                } else {
                    landing_screen()
                }
            })
        })))
    })
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    let state=AppState::new();
    let app=render(state.clone());
    if let Some(document)=web_sys::window().and_then(|w|w.document()) {
        if let Some(root)=document.get_element_by_id("app"){append_dom(&root,app);}
    }
    let s=state.clone();
    spawn_local(async move {
        if let Ok(user)=get_json::<User>("/api/me").await {
            s.user.set(Some(user.clone()));
            load_campaign(s.clone()).await;
            if user.role=="admin" {load_participants(s.clone()).await;} else {load_greetings(s.clone()).await;connect_events(s.clone());}
        }
    });
}
