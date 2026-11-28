use dominator::{append_dom, clone, events, html, routing, Dom};
use futures_signals::{map_ref, signal::{Mutable, SignalExt}};
use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{closure::Closure, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{Element, EventSource, HtmlAudioElement, HtmlElement, HtmlInputElement, HtmlTextAreaElement, MessageEvent, NodeList, WebSocket};

#[derive(Clone, Deserialize, Serialize)]
struct User { subject_id: String, role: String, csrf_token: String }

#[derive(Clone, Debug, PartialEq, Eq)]
enum Route {
    Landing,
    Admin,
    Invitation(String),
}

impl Route {
    fn from_url(url: &str) -> Self {
        // routing::url() is the sole source of navigation state.
        let fragment = url
            .split_once('#')
            .map(|(_, fragment)| fragment)
            .unwrap_or_default();

        if let Some(token) = fragment.strip_prefix("invite=") {
            if !token.is_empty() {
                return Self::Invitation(token.to_owned());
            }
        }

        let path = fragment
            .split('?')
            .next()
            .unwrap_or_default()
            .trim_matches('/');

        match path {
            "admin" => Self::Admin,
            _ => Self::Landing,
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
struct Greeting { ornament_number: i16, display_name: String, message: String }
#[derive(Clone, Deserialize)]
#[allow(dead_code)]
struct Participant {
    ornament_number: i16,
    activated: bool,
    submitted: bool,
    revoked: bool,
    #[serde(default)]
    online: bool,
    #[serde(default)]
    filling: bool,
    display_name: Option<String>,
    created_at: i64,
    used_at: Option<i64>,
}
#[derive(Clone, Deserialize)]
struct CampaignSummary { participant_count: usize, submitted_count: usize }
#[derive(Clone, Deserialize)]
struct CampaignAdminItem { id: String, name: String, created_at: i64, participant_count: usize, submitted_count: usize, #[serde(default)] online_count: usize }
#[derive(Clone, Deserialize)]
struct CreateCampaignResponse { campaign: CampaignAdminItem, invitations: Vec<CreatedInvitation> }
#[derive(Clone, Deserialize)]
struct CreatedInvitation { ornament_number: i16, token: String }
#[derive(Clone)]
struct InvitationLink { ornament_number: i16, url: String }

struct EventSourceConnection {
    source: EventSource,
    handlers: Vec<(&'static str, Closure<dyn FnMut(web_sys::Event)>)>,
}

struct PresenceConnection {
    socket: WebSocket,
    _message_handler: Closure<dyn FnMut(MessageEvent)>,
    _close_handler: Closure<dyn FnMut(web_sys::Event)>,
}

#[derive(Clone)]
struct AppState {
    user: Mutable<Option<User>>,
    auth_ready: Mutable<bool>,
    exchanging: Mutable<bool>,
    greetings: Mutable<Vec<Greeting>>,
    participants: Mutable<Option<Vec<Participant>>>,
    campaigns: Mutable<Option<Vec<CampaignAdminItem>>>,
    selected_campaign: Mutable<Option<CampaignAdminItem>>,
    campaign_participants: Mutable<Option<Vec<Participant>>>,
    campaign_invite_count: Mutable<String>,
    campaign_name: Mutable<String>,
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
    sse_connection: Rc<RefCell<Option<EventSourceConnection>>>,
    presence_connection: Rc<RefCell<Option<PresenceConnection>>>,
    online_count: Mutable<usize>,
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
    logging_in: Mutable<bool>,
    dialog_return_focus: Mutable<Option<web_sys::HtmlElement>>,
}
impl AppState {
    fn new() -> Self {
        Self { user: Mutable::new(None), auth_ready: Mutable::new(false), exchanging: Mutable::new(false), greetings: Mutable::new(Vec::new()), participants: Mutable::new(None), campaigns: Mutable::new(None), selected_campaign: Mutable::new(None), campaign_participants: Mutable::new(None), campaign_invite_count: Mutable::new("5".into()), campaign_name: Mutable::new(String::new()), campaign_count: Mutable::new(0), submitted_count: Mutable::new(0), selected: Mutable::new(None), presentation: Mutable::new(false), presentation_index: Mutable::new(0), presentation_ornament: Mutable::new(None), presentation_paused: Mutable::new(false), music_muted: Mutable::new(false), music_volume: Mutable::new(0.65), audio: Mutable::new(None), sse_connection: Rc::new(RefCell::new(None)), presence_connection: Rc::new(RefCell::new(None)), online_count: Mutable::new(0), name: Mutable::new(String::new()), message: Mutable::new(String::new()), admin_name: Mutable::new(String::new()), admin_password: Mutable::new(String::new()), note: Mutable::new(String::new()), invite_url: Mutable::new(String::new()), invite_urls: Mutable::new(Vec::new()), campaign_size: Mutable::new("20".into()), campaign_creating: Mutable::new(false), submitting: Mutable::new(false), logging_in: Mutable::new(false), dialog_return_focus: Mutable::new(None) }
    }
    fn csrf(&self) -> String { self.user.lock_ref().as_ref().map(|u| u.csrf_token.clone()).unwrap_or_default() }
}
fn trace(msg: &str) {
    web_sys::console::log_1(&format!("[congratis] {msg}").into());
}

async fn fetch_me() -> Result<Option<User>, String> {
    let response = Request::get("/api/me").send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    if status == 401 {
        return Ok(None);
    }
    if !response.ok() {
        return Err(response.text().await.unwrap_or_else(|_| format!("HTTP {status}")));
    }
    response.json::<User>().await.map(Some).map_err(|error| error.to_string())
}

async fn restore_session(state: &AppState, during_exchange: bool) -> Result<(), String> {
    let still_free = |state: &AppState| {
        (during_exchange || !state.exchanging.get()) && state.user.lock_ref().is_none()
    };

    let Some(user) = fetch_me().await? else {
        return Ok(());
    };
    if !still_free(state) {
        return Ok(());
    }

    if user.role == "participant" {
        restore_draft(state, &user.subject_id);
    }
    load_session_data(state.clone(), &user).await;

    // An invitation exchange or another session may have won while data was loading.
    if still_free(state) {
        state.user.set(Some(user));
    }
    Ok(())
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

async fn delete_json(path: &str, body: &impl Serialize, csrf: &str) -> Result<(), String> {
    let response = Request::delete(path)
        .header("content-type", "application/json")
        .header("x-csrf-token", csrf)
        .json(body).map_err(|error| error.to_string())?
        .send().await.map_err(|error| error.to_string())?;
    if response.ok() { Ok(()) } else {
        Err(response.text().await.unwrap_or_else(|_| format!("HTTP {}", response.status())))
    }
}
fn set_note(state: &AppState, value: impl Into<String>) { state.note.set(value.into()); }
fn remember_dialog_trigger(state: &AppState, event: &events::Click) {
    if let Some(target) = event.target().and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok()) {
        state.dialog_return_focus.set(Some(target));
    }
}
fn restore_dialog_focus(state: &AppState) {
    let Some(element) = state.dialog_return_focus.get_cloned() else { return; };
    let Some(window) = web_sys::window() else { return; };
    let callback = Closure::once_into_js(move || {
        let _ = element.focus();
    });
    let _ = window.request_animation_frame(callback.unchecked_ref());
}
fn close_submission_dialog(state: &AppState) {
    if let Some(dialog) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("submission-dialog"))
    {
        if let Ok(dialog) = dialog.dyn_into::<web_sys::HtmlDialogElement>() {
            if dialog.open() { dialog.close(); }
            dialog.remove();
        }
    }
    restore_dialog_focus(state);
}

fn focus_first_dialog_control(dialog_id: &'static str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else { return; };
    let Some(dialog) = document.get_element_by_id(dialog_id) else { return; };
    let Ok(dialog) = dialog.dyn_into::<web_sys::HtmlDialogElement>() else { return; };
    if !dialog.open() {
        let _ = dialog.show_modal();
    }
    if let Ok(Some(control)) = dialog.query_selector(
        "button:not([disabled]), input:not([disabled]), textarea:not([disabled]), a[href], [tabindex]:not([tabindex='-1'])"
    ) {
        if let Ok(control) = control.dyn_into::<web_sys::HtmlElement>() {
            let _ = control.focus();
        }
    }
}
fn schedule_dialog_focus(dialog_id: &'static str) {
    let Some(window) = web_sys::window() else { return; };
    // child_signal replaces the dialog node asynchronously; wait two frames so
    // focus/showModal run against the final node, not the node being removed.
    let callback = Closure::once_into_js(move || {
        let Some(window) = web_sys::window() else { return; };
        let callback = Closure::once_into_js(move || focus_first_dialog_control(dialog_id));
        let _ = window.request_animation_frame(callback.unchecked_ref());
    });
    let _ = window.request_animation_frame(callback.unchecked_ref());
}

fn dialog_focusable_selector() -> String {
    const VISIBLE: &str = ":not([hidden]):not([aria-hidden='true'])";

    [
        "button:not([disabled])",
        "input:not([disabled]):not([type='hidden'])",
        "select:not([disabled])",
        "textarea:not([disabled])",
        "a[href]",
        "summary",
        "[contenteditable='true']",
        "[tabindex]:not([tabindex='-1'])",
    ]
    .iter()
    .map(|selector| format!("{selector}{VISIBLE}"))
    .collect::<Vec<_>>()
    .join(", ")
}

fn html_elements(list: &NodeList) -> Vec<HtmlElement> {
    (0..list.length())
        .filter_map(|i| list.item(i)?.dyn_into().ok())
        .collect()
}

fn same(a: &Element, b: &Element) -> bool {
    a.is_same_node(Some(b.as_ref()))
}

fn trap_dialog_focus(event: &web_sys::KeyboardEvent, dialog_id: &str) -> Option<()> {
    (event.key() == "Tab").then_some(())?;

    let document = web_sys::window()?.document()?;
    let dialog_element = document.get_element_by_id(dialog_id)?;
    let dialog = dialog_element.dyn_into::<web_sys::HtmlDialogElement>().ok()?;
    if !dialog.open() {
        let _ = dialog.show_modal();
    }
    let active = document.active_element()?;

    let items = html_elements(&dialog.query_selector_all(&dialog_focusable_selector()).ok()?);
    let (first, last) = (items.first()?, items.last()?);

    // Shift+Tab уходит с первого на последний, Tab — с последнего на первый.
    let (edge, opposite) = if event.shift_key() {
        (first, last)
    } else {
        (last, first)
    };

    let outside = !dialog.contains(Some(active.as_ref()));
    let must_wrap = outside || same(&active, dialog.as_ref()) || same(&active, edge);

    if must_wrap {
        event.prevent_default();
        let _ = opposite.focus();
    }
    Some(())
}

fn install_dialog_keyboard_handler(state: AppState) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else { return; };
    let callback = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
        move |event: web_sys::KeyboardEvent| {
        if state.selected.get_cloned().is_some() {
            match event.key().as_str() {
                "Escape" => {
                    event.prevent_default();
                    state.selected.set(None);
                    restore_dialog_focus(&state);
                }
                "Tab" => {
                    trap_dialog_focus(&event, "greeting-dialog");
                }
                _ => {}
            }
        } else if web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("submission-dialog"))
            .is_some()
        {
            match event.key().as_str() {
                "Escape" => {
                    event.prevent_default();
                    close_submission_dialog(&state);
                }
                "Tab" => {
                    trap_dialog_focus(&event, "submission-dialog");
                }
                _ => {}
            }
        } else if state.presentation.get() && event.key() == "Escape" {
            event.prevent_default();
            exit_presentation(state.clone());
        }
    });
    if document.add_event_listener_with_callback("keydown", callback.as_ref().unchecked_ref()).is_ok() {
        callback.forget();
    }
}
const PRESENTATION_AUDIO_URL: &str = "/music.mp3";

fn set_presentation_index(state: &AppState, index: usize) {
    let greetings = state.greetings.get_cloned();
    state.presentation_index.set(index);
    state.presentation_ornament.set(greetings.get(index).map(|greeting| greeting.ornament_number));
}

fn play_presentation_audio(state: AppState) {
    let audio = match state.audio.get_cloned() {
        Some(audio) => audio,
        None => match HtmlAudioElement::new_with_src(PRESENTATION_AUDIO_URL) {
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
    restore_dialog_focus(&state);
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
        Ok(items) => state.participants.set(Some(items)),
        Err(error) => set_note(&state, format!("Не удалось загрузить участников: {error}")),
    }
}

async fn load_campaigns(state: AppState) {
    state.campaigns.set(None);
    match get_json::<Vec<CampaignAdminItem>>("/api/admin/campaigns").await {
        Ok(items) => state.campaigns.set(Some(items)),
        Err(error) => {
            state.campaigns.set(Some(Vec::new()));
            set_note(&state, format!("Не удалось загрузить кампании: {error}"));
        }
    }
}

async fn load_campaign_participants(state: AppState, campaign_id: &str) {
    state.campaign_participants.set(None);
    let path = format!("/api/admin/campaigns/{campaign_id}/participants");
    match get_json::<Vec<Participant>>(&path).await {
        Ok(items) => state.campaign_participants.set(Some(items)),
        Err(error) => set_note(&state, format!("Не удалось загрузить участников кампании: {error}")),
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
fn disconnect_events(state: &AppState) {
    let Some(connection) = state.sse_connection.borrow_mut().take() else { return; };
    for (event_name, handler) in &connection.handlers {
        let _ = connection.source.remove_event_listener_with_callback(
            event_name,
            handler.as_ref().unchecked_ref(),
        );
    }
    connection.source.close();
}

fn connect_events(state: AppState) {
    // Replacing a connection must release its listeners before the new EventSource is created.
    disconnect_events(&state);
    let Ok(source) = EventSource::new("/api/events") else { return; };
    let mut handlers = Vec::with_capacity(3);
    // Event callbacks share application data but must not retain the connection that owns them.
    let callback_state = AppState {
        sse_connection: Rc::new(RefCell::new(None)),
        ..state.clone()
    };

    let greeting_state = callback_state.clone();
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
    handlers.push(("greeting", greeting_handler));

    let resync_state = callback_state.clone();
    let resync_handler = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        let state = resync_state.clone();
        spawn_local(async move {
            load_greetings(state.clone()).await;
            load_campaign(state).await;
        });
    });
    let _ = source.add_event_listener_with_callback("resync", resync_handler.as_ref().unchecked_ref());
    handlers.push(("resync", resync_handler));

    // EventSource fires "open" both on the initial connection and after reconnects.
    // Reconcile against the API so a missed SSE notification cannot leave stale progress.
    let open_state = callback_state.clone();
    let open_handler = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        let state = open_state.clone();
        spawn_local(async move {
            load_greetings(state.clone()).await;
            load_campaign(state).await;
        });
    });
    let _ = source.add_event_listener_with_callback("open", open_handler.as_ref().unchecked_ref());
    handlers.push(("open", open_handler));

    *state.sse_connection.borrow_mut() = Some(EventSourceConnection { source, handlers });
}

fn disconnect_presence_events(state: &AppState) {
    let Some(connection) = state.presence_connection.borrow_mut().take() else { return; };
    connection.socket.set_onmessage(None);
    connection.socket.set_onclose(None);
    let _ = connection.socket.close();
}

fn apply_presence_update(state: &AppState, value: serde_json::Value) {
    let kind = value.get("kind").and_then(|v| v.as_str()).unwrap_or_default();
    if let Some(count) = value.get("online_count").and_then(|v| v.as_u64()) {
        state.online_count.set(count as usize);
    }
    if kind != "presence" { return; }
    let campaign_id = value.get("campaign_id").and_then(|v| v.as_str()).unwrap_or_default();
    let number = value.get("ornament_number").and_then(|v| v.as_i64()).unwrap_or_default() as i16;
    let online = value.get("online").and_then(|v| v.as_bool()).unwrap_or(false);
    let filling = value.get("filling").and_then(|v| v.as_bool()).unwrap_or(false);

    if let Some(mut items) = state.participants.get_cloned() {
        if let Some(item) = items.iter_mut().find(|item| item.ornament_number == number) {
            item.online = online;
            item.filling = filling;
        }
        state.participants.set(Some(items));
    }
    if let Some(mut items) = state.campaign_participants.get_cloned() {
        if let Some(item) = items.iter_mut().find(|item| item.ornament_number == number) {
            item.online = online;
            item.filling = filling;
        }
        state.campaign_participants.set(Some(items));
    }
    if let Some(mut items) = state.campaigns.get_cloned() {
        if let Some(item) = items.iter_mut().find(|item| item.id == campaign_id) {
            item.online_count = value.get("online_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        }
        state.campaigns.set(Some(items));
    }
}

fn connect_presence_events(state: AppState) {
    disconnect_presence_events(&state);
    let Some(window) = web_sys::window() else { return; };
    let Ok(protocol) = window.location().protocol() else { return; };
    let Ok(host) = window.location().host() else { return; };
    let scheme = if protocol == "https:" { "wss:" } else { "ws:" };
    let url = format!("{scheme}//{host}/api/presence");
    let Ok(socket) = WebSocket::new(&url) else { return; };

    let message_state = state.clone();
    let message_handler = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        if let Some(raw) = event.data().as_string() {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                apply_presence_update(&message_state, value);
            }
        }
    });
    socket.set_onmessage(Some(message_handler.as_ref().unchecked_ref()));

    let close_state = state.clone();
    let close_handler = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        let retry_state = close_state.clone();
        let callback = Closure::once_into_js(move || connect_presence_events(retry_state));
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(callback.unchecked_ref(), 2500);
        }
    });
    socket.set_onclose(Some(close_handler.as_ref().unchecked_ref()));
    *state.presence_connection.borrow_mut() = Some(PresenceConnection {
        socket, _message_handler: message_handler, _close_handler: close_handler
    });
}

fn send_presence_activity(state: &AppState) {
    if let Some(connection) = state.presence_connection.borrow().as_ref() {
        if connection.socket.ready_state() == WebSocket::OPEN {
            let _ = connection.socket.send_with_str(r#"{"type":"activity"}"#);
        }
    }
}

fn draft_storage_key(subject_id: &str) -> String {
    format!("congratis:draft:{subject_id}")
}
fn restore_draft(state: &AppState, subject_id: &str) {
    let Some(storage) = web_sys::window().and_then(|window| window.local_storage().ok().flatten()) else { return; };
    let Ok(Some(raw)) = storage.get_item(&draft_storage_key(subject_id)) else { return; };
    let Ok(draft) = serde_json::from_str::<serde_json::Value>(&raw) else { return; };
    if let Some(name) = draft.get("display_name").and_then(serde_json::Value::as_str) {
        state.name.set(name.to_owned());
    }
    if let Some(message) = draft.get("message").and_then(serde_json::Value::as_str) {
        state.message.set(message.to_owned());
    }
}
fn persist_draft(state: &AppState, subject_id: &str) {
    let Some(storage) = web_sys::window().and_then(|window| window.local_storage().ok().flatten()) else { return; };
    let draft = serde_json::json!({
        "display_name": state.name.get_cloned(),
        "message": state.message.get_cloned()
    });
    if let Ok(raw) = serde_json::to_string(&draft) {
        let _ = storage.set_item(&draft_storage_key(subject_id), &raw);
    }
}
fn clear_draft(subject_id: &str) {
    if let Some(storage) = web_sys::window().and_then(|window| window.local_storage().ok().flatten()) {
        let _ = storage.remove_item(&draft_storage_key(subject_id));
    }
}
fn field(
    label: &'static str,
    placeholder: &'static str,
    value: Mutable<String>,
    is_textarea: bool,
    draft_state: Option<AppState>,
    draft_subject_id: Option<String>,
) -> Dom {
    if is_textarea {
        html!("label", { .text(label).child(html!("textarea", {
            .attr("rows", "4").attr("maxlength", "500").attr("placeholder", placeholder)
            .property("value", &value.get_cloned())
            .event(clone!(value, draft_state, draft_subject_id => move |event: events::Input| {
                if let Some(target) = event.target() {
                    if let Ok(input) = target.dyn_into::<HtmlTextAreaElement>() {
                        value.set(input.value());
                        if let Some(state) = &draft_state { send_presence_activity(state); }
                        if let (Some(state), Some(subject_id)) = (&draft_state, &draft_subject_id) {
                            persist_draft(state, subject_id);
                        }
                    }
                }
            }))
        })) })
    } else {
        html!("label", { .text(label).child(html!("input", {
            .attr("type", "text").attr("maxlength", "80").attr("placeholder", placeholder)
            .property("value", &value.get_cloned())
            .event(clone!(value, draft_state, draft_subject_id => move |event: events::Input| {
                if let Some(target) = event.target() {
                    if let Ok(input) = target.dyn_into::<HtmlInputElement>() {
                        value.set(input.value());
                        if let (Some(state), Some(subject_id)) = (&draft_state, &draft_subject_id) {
                            persist_draft(state, subject_id);
                        }
                    }
                }
            }))
        })) })
    }
}
fn ipsen_brand() -> Dom {
    html!("a", {
        .class("ipsen-brand")
        .attr("href", "https://www.ipsen.com/")
        .attr("target", "_blank")
        .attr("rel", "noopener noreferrer")
        .attr("aria-label", "Ipsen — официальный сайт")
        .child(html!("img", {
            .attr("src", "https://www.ipsen.com/wp-content/uploads/2023/12/Latest_Ipsen-logo_168x48.svg")
            .attr("alt", "Ipsen")
            .attr("width", "168")
            .attr("height", "48")
        }))
    })
}

fn participant_tree(state: AppState, user: User) -> Dom {
    let is_mine = user.role == "participant";
    let pill_label = if is_mine { format!("Твой шар №{}", user.subject_id) } else { "Режим администратора".to_string() };
    html!("main", {
        .class("shell")
        .class_signal("authenticated-shell", futures_signals::signal::always(is_mine))
        .child_signal(futures_signals::signal::always(is_mine).map(|visible| visible.then(ipsen_brand)))
        .child(html!("div", {.class("eyebrow").text("A LITTLE CHRISTMAS MAGIC")}))
        .child(html!("h1", {.text("Ёлка добрых ").child(html!("span", {.text("пожеланий")}))}))
        .child(html!("p", {.class("team-subtitle").text("команда редких заболеваний")}))
        .child(html!("p", {.class("lead").text("Выбери шар — внутри тебя ждёт новогоднее поздравление. Все пожелания команды собраны на одной ёлке.")}))
        .child(html!("div", {
            .class("toolbar")
             .child(html!("span", {
                .class("pill")
                .text(&pill_label)
            }))
            .child(html!("span", {
                .class("pill online-count")
                .attr("aria-label", "Участников онлайн")
                .text_signal(state.online_count.signal().map(|count| format!("● {count} онлайн")))
            }))
            .child_signal(futures_signals::signal::always(!is_mine).map(|visible| {
                visible.then(|| html!("button", {
                    .class("secondary")
                    .text("Админка")
                    .event(|_: events::Click| routing::go_to_url("/#/admin"))
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
                                reset_session_state(&s);
                                routing::go_to_url("/");
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
                        .event(clone!(state, greeting => move |event: events::Click| {
                            if let Some(g)=greeting.clone() {
                                remember_dialog_trigger(&state, &event);
                                state.selected.set(Some(g));
                                schedule_dialog_focus("greeting-dialog");
                            }
                            else if mine {
                                remember_dialog_trigger(&state, &event);
                                show_form(state.clone());
                            }
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
                                 .event(clone!(button_state => move |event: events::Click| {
                                    remember_dialog_trigger(&button_state, &event);
                                    start_presentation(button_state.clone());
                                    schedule_dialog_focus("presentation-dialog");
                                }))
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
                html!("dialog", {
                    .class("overlay")
                    .attr("id", "greeting-dialog")
                    .attr("aria-labelledby", "greeting-title")
                    .child(html!("section", {
                        .class("dialog")
                        
                        .child(html!("button", {
                            .class("close")
                            .attr("type", "button")
                            .attr("aria-label", "Закрыть поздравление")
                            .attr("autofocus", "")
                            .text("×")
                                                        .event(clone!(state => move |_: events::Click| {
                                state.selected.set(None);
                                restore_dialog_focus(&state);
                            }))
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
                                    .event(clone!(state => move |_: events::Click| {
                                        state.selected.set(Some(item.clone()));
                                        schedule_dialog_focus("greeting-dialog");
                                    }))
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
                                                        .event(clone!(state => move |_: events::Click| {
                                state.selected.set(None);
                                restore_dialog_focus(&state);
                            }))
                        }))
                    }))
                })
            })
        })))
        .child_signal(state.presentation.signal().map(clone!(state => move |active| {
            if !active {
                None
            } else {
                Some(html!("dialog", {
                    .class("presentation-overlay")
                    .attr("id", "presentation-dialog")
                    .attr("aria-labelledby", "presentation-title")
                    .child(html!("section", {
                        .class("presentation-card")
                        
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
                            let finale = Mutable::new(index == count && count > 0);
                            Some(html!("div", {
                                .class("presentation-content")
                                .class_signal("presentation-finale", finale.signal())
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
                                .attr("autofocus", "")
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
                    }))
                }))
            }
        })))
    })
}
fn show_form(state: AppState) {
    let form_state = state.clone();
    let subject_id = state.user.lock_ref().as_ref()
        .map(|user| user.subject_id.clone()).unwrap_or_default();
    let dialog = html!("dialog", {
        .class("overlay")
        .attr("id", "submission-dialog")
        .attr("aria-labelledby", "submission-title")
        .event(clone!(form_state => move |event: events::KeyDown| {
            if event.key() == "Escape" {
                event.prevent_default();
                close_submission_dialog(&form_state);
            }
        }))
        .child(html!("section", {
            .class("dialog")
            
            .child(html!("button", {
                .class("close")
                .attr("type", "button")
                .attr("aria-label", "Закрыть форму поздравления")
                .attr("autofocus", "")
                .text("×")
                .event(clone!(form_state => move |_: events::Click| {
                    close_submission_dialog(&form_state);
                }))
            }))
            .child(html!("div", {
                .class("kicker")
                .text("YOUR PERSONAL ORNAMENT")
            }))
            .child(html!("h2", {
                .attr("id", "submission-title")
                .text("Добавь своё пожелание")
            }))
            .child(html!("p", {
                .class("muted")
                .text("Твоё приглашение одноразовое. После публикации поздравление появится у всей команды.")
            }))
            .child(field("Как тебя зовут?", "Твоё имя", state.name.clone(), false, Some(form_state.clone()), Some(subject_id.clone())))
            .child(field("Твоё поздравление", "Пусть в новом году...", state.message.clone(), true, Some(form_state.clone()), Some(subject_id.clone())))
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
                    let subject_id = current.user.lock_ref().as_ref().map(|user| user.subject_id.clone()).unwrap_or_default();
                    spawn_local(async move {
                        let body = serde_json::json!({
                            "display_name": display_name,
                            "message": message
                        });
                        match post_json::<serde_json::Value>("/api/greetings", &body, Some(&csrf)).await {
                            Ok(_) => {
                                set_note(&current, "Поздравление опубликовано! ✨");
                                clear_draft(&subject_id);
                                current.name.set(String::new());
                                current.message.set(String::new());
                                load_greetings(current.clone()).await;
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
            schedule_dialog_focus("submission-dialog");
        }
    }
}

fn create_named_campaign_panel(state: AppState) -> Dom {
    html!("section", {
        .class("panel")
        .class("admin-create-campaign")
        .child(html!("div", {.class("kicker").text("NEW CAMPAIGN")}))
        .child(html!("h2", {.text("Создать кампанию")}))
        .child(html!("label", {
            .text("Название кампании")
            .child(html!("input", {
                .attr("type", "text").attr("maxlength", "100")
                .attr("aria-label", "Название кампании")
                .property("value", &state.campaign_name.get_cloned())
                .event(clone!(state => move |event: events::Input| {
                    if let Some(target) = event.target() {
                        if let Ok(input) = target.dyn_into::<HtmlInputElement>() { state.campaign_name.set(input.value()); }
                    }
                }))
            }))
        }))
        .child(html!("label", {
            .text("Количество участников (от 1 до 35)")
            .child(html!("input", {
                .attr("type", "number").attr("min", "1").attr("max", "35").attr("step", "1")
                .attr("aria-label", "Размер новой кампании")
                .property("value", &state.campaign_size.get_cloned())
                .event(clone!(state => move |event: events::Input| {
                    if let Some(target) = event.target() {
                        if let Ok(input) = target.dyn_into::<HtmlInputElement>() { state.campaign_size.set(input.value()); }
                    }
                }))
            }))
        }))
        .child(html!("button", {
            .class("primary")
            .text_signal(state.campaign_creating.signal().map(|creating| if creating { "Создаём кампанию…" } else { "Создать именованную кампанию" }))
            .event(clone!(state => move |_: events::Click| {
                if state.campaign_creating.get() { return; }
                let name = state.campaign_name.get_cloned();
                if name.trim().is_empty() || name.chars().count() > 100 {
                    set_note(&state, "Название должно содержать от 1 до 100 символов"); return;
                }
                let count = match state.campaign_size.get_cloned().parse::<i16>() {
                    Ok(count) if (1..=35).contains(&count) => count,
                    _ => { set_note(&state, "Количество участников должно быть от 1 до 35"); return; }
                };
                state.campaign_creating.set(true);
                let csrf = state.csrf();
                let current = state.clone();
                spawn_local(async move {
                    let body = serde_json::json!({ "name": name, "participant_count": count });
                    match post_json::<CreateCampaignResponse>("/api/admin/campaigns", &body, Some(&csrf)).await {
                        Ok(created) => {
                            let origin = web_sys::window().map(|window| window.location().origin().unwrap_or_default()).unwrap_or_default();
                            let links = created.invitations.into_iter().map(|invitation| InvitationLink {
                                ornament_number: invitation.ornament_number,
                                url: format!("{origin}/#invite={}", invitation.token),
                            }).collect();
                            current.invite_urls.set(links);
                            current.campaign_name.set(String::new());
                            set_note(&current, format!("Кампания «{}» создана. Сохраните одноразовые ссылки.", created.campaign.name));
                            load_campaigns(current.clone()).await;
                        }
                        Err(error) => set_note(&current, error),
                    }
                    current.campaign_creating.set(false);
                });
            }))
        }))
    })
}

fn campaign_card(state: AppState, campaign: CampaignAdminItem) -> Dom {
    let id = campaign.id.clone();
    let name = campaign.name.clone();
    let protected = id == "legacy-default";
    let campaign_for_manage = campaign.clone();
    html!("article", {
        .class("admin-item")
        .child(html!("strong", {.text(&campaign.name)}))
        .child(html!("small", {.text(&format!("Участников: {} · Онлайн: {} · Поздравлений: {}", campaign.participant_count, campaign.online_count, campaign.submitted_count))}))
        .child(html!("small", {.text(if protected { "Системная кампания — удаление запрещено" } else { "Кампания доступна для управления" })}))
        .child(if protected {
            html!("span", {.class("muted").text("Системная кампания — защищена")})
        } else {
            html!("button", {
                .class("secondary")
                .text("Участники и приглашения")
                .event(clone!(state => move |_: events::Click| {
                    state.selected_campaign.set(Some(campaign_for_manage.clone()));
                    state.campaign_invite_count.set("5".into());
                    let current = state.clone();
                    let campaign_id = campaign_for_manage.id.clone();
                    spawn_local(async move { load_campaign_participants(current, &campaign_id).await; });
                }))
            })
        })
        .child(if protected {
            html!("span", {.class("muted").text("Удаление запрещено")})
        } else {
            html!("button", {
                .class("secondary")
                .text("Удалить кампанию")
                .event(clone!(state => move |_: events::Click| {
                    let prompt = format!("Удаление кампании «{name}» необратимо удалит её приглашения, участников, сессии и поздравления.\\n\\nВведите точное название кампании для подтверждения:");
                    let confirmation = web_sys::window().and_then(|window| window.prompt_with_message(&prompt).ok()).flatten();
                    let Some(confirmation) = confirmation else { return; };
                    let csrf = state.csrf();
                    let current = state.clone();
                    let path = format!("/api/admin/campaigns/{id}");
                    let deleted_name = name.clone();
                    let deleted_id = id.clone();
                    spawn_local(async move {
                        let body = serde_json::json!({ "confirm_name": confirmation });
                        match delete_json(&path, &body, &csrf).await {
                            Ok(()) => {
                                if current.selected_campaign.lock_ref().as_ref().is_some_and(|item| item.id == deleted_id) {
                                    current.selected_campaign.set(None);
                                    current.campaign_participants.set(None);
                                }
                                set_note(&current, format!("Кампания «{deleted_name}» удалена."));
                                load_campaigns(current.clone()).await;
                            }
                            Err(error) => set_note(&current, error),
                        }
                    });
                }))
            })
        })
    })
}

fn campaigns_panel(state: AppState) -> Dom {
    html!("section", {
        .class("admin-invite-list")
        .child(html!("h2", {.text("Кампании")}))
        .child_signal(state.campaigns.signal_cloned().map(clone!(state => move |campaigns| {
            Some(match campaigns {
                None => html!("p", {.class("muted").text("Загружаем кампании…")}),
                Some(items) if items.is_empty() => html!("p", {.class("muted").text("Кампании не найдены.")}),
                Some(items) => html!("div", {
                    .class("admin-grid")
                    .children(items.into_iter().map(clone!(state => move |campaign| campaign_card(state.clone(), campaign))))
                })
            })
        })))
    })
}

fn campaign_participants_grid(state: AppState, campaign_id: String, participants: Vec<Participant>) -> Dom {
    html!("div", {
        .class("admin-grid")
        .children(participants.into_iter().map(clone!(state, campaign_id => move |participant| {
            let number = participant.ornament_number;
            let display_name = participant.display_name.clone().unwrap_or_default();
            let status = if participant.submitted {
                "Поздравление опубликовано"
            } else if participant.filling {
                "Заполняет приглашение сейчас"
            } else if participant.online {
                "Сейчас онлайн"
            } else if participant.activated {
                "Приглашение активировано"
            } else if participant.revoked {
                "Приглашение отозвано"
            } else {
                "Ожидает участника"
            };
            html!("article", {
                .class("admin-item")
                .child(html!("strong", {.text(&format!("Шар №{number}"))}))
                .child(html!("small", {.text(status)}))
                .child(html!("small", {.text(&display_name)}))
                .children(if !participant.activated && !participant.submitted && !participant.revoked {
                    vec![
                        html!("button", {
                            .class("secondary")
                            .text("Перевыпустить ссылку")
                            .event(clone!(state, campaign_id => move |_: events::Click| {
                                let csrf = state.csrf();
                                let current = state.clone();
                                let path = format!("/api/admin/campaigns/{campaign_id}/invitations/{number}/reissue");
                                let refresh_campaign_id = campaign_id.clone();
                                spawn_local(async move {
                                    match post_json::<serde_json::Value>(&path, &serde_json::json!({}), Some(&csrf)).await {
                                        Ok(data) => {
                                            let token = data["token"].as_str().unwrap_or_default();
                                            let origin = web_sys::window().map(|window| window.location().origin().unwrap_or_default()).unwrap_or_default();
                                            let url = format!("{origin}/#invite={token}");
                                            current.invite_url.set(url.clone());
                                            set_note(&current, format!("Одноразовая ссылка для шара №{number}: {url}"));
                                            load_campaign_participants(current.clone(), &refresh_campaign_id).await;
                                        }
                                        Err(error) => set_note(&current, error),
                                    }
                                });
                            }))
                        }),
                        html!("button", {
                            .class("secondary")
                            .text("Отозвать")
                            .event(clone!(state, campaign_id => move |_: events::Click| {
                                let confirmed = web_sys::window().and_then(|window| window.confirm_with_message(
                                    &format!("Отозвать приглашение для шара №{number}? Ссылка перестанет работать.")
                                ).ok()).unwrap_or(false);
                                if !confirmed { return; }
                                let csrf = state.csrf();
                                let current = state.clone();
                                let path = format!("/api/admin/campaigns/{campaign_id}/invitations/{number}/revoke");
                                let refresh_campaign_id = campaign_id.clone();
                                spawn_local(async move {
                                    match post_empty(&path, &csrf).await {
                                        Ok(()) => {
                                            load_campaign_participants(current.clone(), &refresh_campaign_id).await;
                                            load_campaigns(current.clone()).await;
                                        }
                                        Err(error) => set_note(&current, error),
                                    }
                                });
                            }))
                        }),
                    ]
                } else { vec![] })
            })
        })))
    })
}

fn campaign_participants_panel(state: AppState) -> Dom {
    html!("section", {
        .class("admin-invite-list")
        .child_signal(state.selected_campaign.signal_cloned().map(clone!(state => move |selected| {
            selected.map(|campaign| {
                let campaign_id = campaign.id.clone();
                html!("div", {
                    .child(html!("h2", {.text(&format!("Участники: {}", campaign.name))}))
                    .child(html!("button", {
                        .class("secondary")
                        .text("К списку кампаний")
                        .event(clone!(state => move |_: events::Click| {
                            state.selected_campaign.set(None);
                            state.campaign_participants.set(None);
                        }))
                    }))
                    .child(html!("div", {
                        .class("campaign-add-invitations")
                        .child(html!("label", {
                            .text("Количество новых приглашений (1–35)")
                            .child(html!("input", {
                                .attr("type", "number").attr("min", "1").attr("max", "35").attr("step", "1")
                                .attr("aria-label", "Количество новых приглашений")
                                .property("value", &state.campaign_invite_count.get_cloned())
                                .event(clone!(state => move |event: events::Input| {
                                    if let Some(target) = event.target() {
                                        if let Ok(input) = target.dyn_into::<HtmlInputElement>() { state.campaign_invite_count.set(input.value()); }
                                    }
                                }))
                            }))
                        }))
                        .child(html!("button", {
                            .class("primary")
                            .text("Добавить приглашения")
                            .event(clone!(state, campaign_id => move |_: events::Click| {
                                let count = match state.campaign_invite_count.get_cloned().parse::<i16>() {
                                    Ok(value) if (1..=35).contains(&value) => value,
                                    _ => { set_note(&state, "Количество новых приглашений должно быть от 1 до 35"); return; }
                                };
                                let csrf = state.csrf();
                                let current = state.clone();
                                let path = format!("/api/admin/campaigns/{campaign_id}/invitations");
                                let refresh_campaign_id = campaign_id.clone();
                                spawn_local(async move {
                                    let body = serde_json::json!({ "count": count });
                                    match post_json::<Vec<CreatedInvitation>>(&path, &body, Some(&csrf)).await {
                                        Ok(created) => {
                                            let origin = web_sys::window().map(|window| window.location().origin().unwrap_or_default()).unwrap_or_default();
                                            let links = created.into_iter().map(|invitation| InvitationLink {
                                                ornament_number: invitation.ornament_number,
                                                url: format!("{origin}/#invite={}", invitation.token),
                                            }).collect();
                                            current.invite_urls.set(links);
                                            set_note(&current, "Новые приглашения созданы. Сохраните одноразовые ссылки.");
                                            load_campaign_participants(current.clone(), &refresh_campaign_id).await;
                                            load_campaigns(current.clone()).await;
                                        }
                                        Err(error) => set_note(&current, error),
                                    }
                                });
                            }))
                        }))
                    }))
                    .child_signal(state.campaign_participants.signal_cloned().map(clone!(state, campaign_id => move |participants| {
                        Some(match participants {
                            None => html!("p", {.class("muted").text("Загружаем участников…")}),
                            Some(items) => campaign_participants_grid(state.clone(), campaign_id.clone(), items),
                        })
                    })))
                })
            })
        })))
    })
}

fn participants_grid(state: AppState, participants: Vec<Participant>) -> Dom {
    html!("div", {
        .class("admin-grid")
        .children(participants.into_iter().map(clone!(state => move |participant| {
            let number = participant.ornament_number;
            let display_name = participant.display_name.unwrap_or_default();
            let status = if participant.submitted {
                "Поздравление опубликовано"
            } else if participant.filling {
                "Заполняет приглашение сейчас"
            } else if participant.online {
                "Сейчас онлайн"
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
                                    match post_json::<serde_json::Value>(&path, &serde_json::json!({}), Some(&csrf)).await {
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
                                let confirmed = web_sys::window()
                                    .and_then(|window| window.confirm_with_message(
                                        &format!("Отозвать приглашение для шара №{number}? Ссылка перестанет работать.")
                                    ).ok())
                                    .unwrap_or(false);
                                if !confirmed { return; }
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
                        }),
                    ]
                } else { vec![] })
            })
        })))
    })
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
                    routing::go_to_url("/");
                })
            }))
            .child(html!("button", {
                .class("secondary")
                .text("Обновить")
                .event(clone!(state => move |_: events::Click| {
                    let current = state.clone();
                    spawn_local(async move {
                        load_participants(current.clone()).await;
                        load_campaigns(current).await;
                    });
                }))
            }))
        }))
        .child(create_named_campaign_panel(state.clone()))
        .child(campaigns_panel(state.clone()))
        .child(campaign_participants_panel(state.clone()))
        .child_signal(state.participants.signal_cloned().map(clone!(state => move |participants| {
            Some(match participants {
                None => html!("section", {
                    .class("panel")
                    .attr("aria-busy", "true")
                    .child(html!("p", {.class("muted").text("Загружаем участников…")}))
                }),
                Some(list) if list.is_empty() => html!("section", {
                    .class("panel")
                    .child(html!("p", {
                        .class("muted")
                        .text("В системной кампании пока нет участников. Создайте отдельную кампанию в форме выше.")
                    }))
                }),
                Some(list) => participants_grid(state.clone(), list),
            })
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
            .child(field("Логин", "admin", state.admin_name.clone(), false, None, None))
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
                .text_signal(state.logging_in.signal().map(|logging_in| {
                    if logging_in { "Входим…" } else { "Войти" }
                }))
                .event(clone!(state => move |_: events::Click| {
                    if state.logging_in.replace(true) { return; }
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
                                set_note(&current, "");
                                current.participants.set(None);
                                load_session_data(current.clone(), &user).await;
                                current.admin_password.set(String::new());
                                current.user.set(Some(user));
                                current.logging_in.set(false);
                            }
                            Err(error) => {
                                current.logging_in.set(false);
                                set_note(&current, error);
                            }
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
fn loading_screen() -> Dom {
    html!("main", {
        .class("shell")
        .attr("aria-busy", "true")
        .child(html!("h1", {.text("Загрузка…")}))
    })
}

fn landing_screen(state: AppState) -> Dom {
    html!("main", {
        .class("shell")
        .child(html!("div", {.class("eyebrow").text("A LITTLE CHRISTMAS MAGIC")}))
        .child(html!("h1", {
            .text("Ёлка добрых ")
            .child(html!("span", {.text("пожеланий")}))
        }))
        .child(html!("div", {
            .class("no-invitation-art")
            .child(html!("img", {
                .attr("src", "/no-invitation.webp")
                .attr("alt", "У Вас нет приглашения? Свяжитесь с администратором.")
                .attr("loading", "eager")
                .attr("decoding", "async")
            }))
        }))
        .child(html!("div", {.class("note").text_signal(state.note.signal_cloned())}))
        .child(html!("button", {
            .class("primary")
            .text("Вход администратора")
            .event(|_: events::Click| routing::go_to_url("/#/admin"))
        }))
    })
}

fn reset_session_state(state: &AppState) {
    disconnect_events(state);
    disconnect_presence_events(state);
    if let Some(audio) = state.audio.get_cloned() {
        audio.pause();
    }
    state.audio.set(None);

    if let Some(dialog) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("submission-dialog"))
    {
        dialog.remove();
    }

    state.user.set(None);
    state.greetings.set(Vec::new());
    state.participants.set(None);
    state.campaigns.set(None);
    state.selected_campaign.set(None);
    state.campaign_participants.set(None);
    state.campaign_invite_count.set("5".into());
    state.campaign_name.set(String::new());
    state.campaign_count.set(0);
    state.submitted_count.set(0);
    state.online_count.set(0);
    state.selected.set(None);
    state.presentation.set(false);
    state.presentation_paused.set(false);
    state.presentation_index.set(0);
    state.presentation_ornament.set(None);
    state.invite_url.set(String::new());
    state.invite_urls.set(Vec::new());
    state.name.set(String::new());
    state.message.set(String::new());
    state.admin_name.set(String::new());
    state.admin_password.set(String::new());
    state.campaign_creating.set(false);
    state.submitting.set(false);
    state.logging_in.set(false);
    state.note.set(String::new());
}

async fn load_session_data(state: AppState, user: &User) {
    trace("data: loading campaign");
    load_campaign(state.clone()).await;
    trace("data: campaign loaded");
    load_greetings(state.clone()).await;
    trace("data: greetings loaded");
    if user.role == "admin" {
        load_participants(state.clone()).await;
        load_campaigns(state.clone()).await;
        trace("data: participants and campaigns loaded");
    }
    connect_events(state.clone());
    connect_presence_events(state);
    trace("data: SSE and presence WebSockets connected");
}

fn begin_invitation(state: AppState, token: String) {
    if state.exchanging.replace(true) {
        trace("invite: exchange skipped (already exchanging)");
        return;
    }

    spawn_local(async move {
        trace("invite: exchange started");
        let body = serde_json::json!({ "token": token });
        let result = post_json::<User>("/api/auth/participant/exchange", &body, None).await;

        // The invitation token is single-use and is no longer needed after the exchange attempt.
        // Always leave the invitation route so a failed exchange cannot be restarted by watch_routes.
        routing::replace_url("/");

        match result {
            Ok(user) => {
                trace("exchange ok");
                reset_session_state(&state);
                trace("session reset");
                restore_draft(&state, &user.subject_id);
                state.user.set(Some(user.clone()));
                trace("user set");
                load_session_data(state.clone(), &user).await;
                trace("data loaded");
            }
            Err(error) => {
                trace(&format!("exchange error: {error}"));
                set_note(&state, format!("Не удалось активировать приглашение: {error}"));
                // Keep exchanging=true while restoring a pre-existing cookie session,
                // so the landing page is not rendered between the failed exchange and recovery.
                let _ = restore_session(&state, true).await;
                trace("error: session restore finished");
            }
        }
        state.auth_ready.set_neq(true);
        state.exchanging.set(false);
        trace("done");
    });
}

fn watch_routes(state: AppState) {
    let signal = map_ref! {
        let url = routing::url().signal_cloned(),
        let user = state.user.signal_cloned(),
        let ready = state.auth_ready.signal() => {
            (Route::from_url(&url), user.clone(), *ready)
        }
    };

    spawn_local(signal.for_each(move |(route, user, ready)| {
        match (&route, &user) {
            (Route::Invitation(token), _) => begin_invitation(state.clone(), token.clone()),
            (Route::Admin, Some(user)) if ready && user.role != "admin" => {
                routing::replace_url("/");
            }
            _ => {}
        }
        async {}
    }));
}

fn render_route(state: AppState, route: Route, user: Option<User>, ready: bool) -> Dom {
    if !ready {
        return loading_screen();
    }

    match user {
        Some(user) if user.role == "admin" => match route {
            Route::Admin => admin_screen(state),
            Route::Landing | Route::Invitation(_) => participant_tree(state, user),
        },
        Some(user) => participant_tree(state, user),
        None => match route {
            Route::Admin => admin_login(state),
            Route::Landing | Route::Invitation(_) => landing_screen(state),
        },
    }
}

fn render(state: AppState) -> Dom {
    let render_state = state.clone();
    let view = map_ref! {
        let url = routing::url().signal_cloned(),
        let user = state.user.signal_cloned(),
        let ready = state.auth_ready.signal(),
        let exchanging = state.exchanging.signal() => {
            let show_ready = *ready && !*exchanging;
            Some(render_route(
                render_state.clone(),
                Route::from_url(&url),
                user.clone(),
                show_ready,
            ))
        }
    };

    html!("div", {
        .child_signal(view)
    })
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    let state = AppState::new();
    install_dialog_keyboard_handler(state.clone());
    watch_routes(state.clone());

    let app = render(state.clone());
    if let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("app"))
    {
        append_dom(&root, app);
    }

    let current = state.clone();
    spawn_local(async move {
        if !current.exchanging.get() {
            if let Err(error) = restore_session(&current, false).await {
                if !current.exchanging.get() {
                    set_note(&current, format!("Не удалось проверить сессию: {error}"));
                }
            }
        }
        current.auth_ready.set_neq(true);
    });
}
