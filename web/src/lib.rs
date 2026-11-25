use dominator::{append_dom, clone, events, html, Dom};
use futures_signals::signal::{Mutable, SignalExt};
use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{closure::Closure, JsCast};
use wasm_bindgen_futures::spawn_local;
use web_sys::{EventSource, HtmlInputElement, HtmlTextAreaElement, MessageEvent};

#[derive(Clone, Deserialize, Serialize)]
struct User { subject_id: String, role: String, csrf_token: String }
#[derive(Clone, Deserialize, Serialize)]
struct Greeting { ornament_number: i16, display_name: String, message: String }
#[derive(Clone, Deserialize)]
struct Participant { ornament_number: i16, activated: bool, submitted: bool, revoked: bool, display_name: Option<String>, created_at: i64, used_at: Option<i64> }

#[derive(Clone)]
struct AppState {
    user: Mutable<Option<User>>,
    greetings: Mutable<Vec<Greeting>>,
    participants: Mutable<Vec<Participant>>,
    selected: Mutable<Option<Greeting>>,
    name: Mutable<String>,
    message: Mutable<String>,
    admin_name: Mutable<String>,
    admin_password: Mutable<String>,
    note: Mutable<String>,
    invite_url: Mutable<String>,
}
impl AppState {
    fn new() -> Self {
        Self { user: Mutable::new(None), greetings: Mutable::new(Vec::new()), participants: Mutable::new(Vec::new()), selected: Mutable::new(None), name: Mutable::new(String::new()), message: Mutable::new(String::new()), admin_name: Mutable::new(String::new()), admin_password: Mutable::new(String::new()), note: Mutable::new(String::new()), invite_url: Mutable::new(String::new()) }
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
async fn load_greetings(state: AppState) {
    match get_json::<Vec<Greeting>>("/api/greetings").await {
        Ok(items) => state.greetings.set(items),
        Err(error) => set_note(&state, format!("Не удалось загрузить поздравления: {error}")),
    }
}
async fn load_participants(state: AppState) {
    match get_json::<Vec<Participant>>("/api/admin/participants").await {
        Ok(items) => state.participants.set(items),
        Err(error) => set_note(&state, format!("Не удалось загрузить участников: {error}")),
    }
}
fn connect_events(state: AppState) {
    let Ok(source) = EventSource::new("/api/events") else { return; };
    let greeting_state = state.clone();
    let greeting_handler = Closure::<dyn FnMut(web_sys::Event)>::new(move |event| {
        if let Ok(message) = event.dyn_into::<MessageEvent>() {
            if let Some(raw) = message.data().as_string() {
                if let Ok(greeting) = serde_json::from_str::<Greeting>(&raw) {
                    let mut all = greeting_state.greetings.lock_mut();
                    if let Some(old) = all.iter_mut().find(|g| g.ornament_number == greeting.ornament_number) { *old = greeting; }
                    else { all.push(greeting); }
                }
            }
        }
    });
    let _ = source.add_event_listener_with_callback("greeting", greeting_handler.as_ref().unchecked_ref());
    greeting_handler.forget();
    let resync_state = state.clone();
    let resync_handler = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        let state = resync_state.clone();
        spawn_local(async move { load_greetings(state).await; });
    });
    let _ = source.add_event_listener_with_callback("resync", resync_handler.as_ref().unchecked_ref());
    resync_handler.forget();
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
    let points: [(f64,f64);20] = [(50.2,18.3),(46.2,27.9),(55.1,28.7),(41.5,38.1),(50.5,38.5),(59.1,38.6),(37.7,49.8),(46.5,50.3),(55.2,50.5),(63.8,50.7),(35.9,60.4),(45.0,61.0),(55.3,61.1),(64.5,61.6),(32.3,72.5),(42.7,73.2),(56.0,73.6),(66.0,73.3),(45.4,85.2),(59.7,85.5)];
    let is_mine = user.role == "participant";
    html!("main", {
        .class("shell")
        .child(html!("div", {.class("eyebrow").text("A LITTLE CHRISTMAS MAGIC")}))
        .child(html!("h1", {.text("Ёлка добрых ").child(html!("span", {.text("пожеланий")}))}))
        .child(html!("p", {.class("lead").text("Выбери шар — внутри тебя ждёт новогоднее поздравление. Все пожелания команды собраны на одной ёлке.")}))
        .child(html!("div", {.class("toolbar")
            .child(html!("span", {.class("pill").text(if is_mine { format!("Твой шар №{}", user.subject_id) } else { "Режим администратора".into() }}))
            .child(html!("button", {.class("secondary").text("Админка").event(clone!(state => move |_| { if let Some(window)=web_sys::window(){ let _=window.location().set_hash("/admin"); let _=window.location().reload(); } })))}))
            .child(html!("button", {.class("secondary").text("Выйти").event(clone!(state => move |_| { let csrf=state.csrf(); let s=state.clone(); spawn_local(async move { match post_empty("/api/auth/logout",&csrf).await { Ok(()) => { s.user.set(None); if let Some(w)=web_sys::window(){let _=w.location().set_hash("");let _=w.location().reload();} }, Err(e)=>set_note(&s,e) } }); })))}))
        }))
        .child(html!("div", {.class("tree-frame")
            .child(html!("img", {.class("tree-art").attr("src","/christmas-tree.webp").attr("alt","Иллюстрированная новогодняя ёлка с двадцатью шарами")}))
            .child_signal(state.greetings.signal_cloned().map(clone!(state => move |greeting_snapshot| {
                html!("div", {.class("hotspots").children(points.iter().enumerate().map(|(index,(x,y))| {
                    let number=(index+1) as i16;
                    let greeting=greeting_snapshot.iter().find(|g|g.ornament_number==number).cloned();
                    let mine=is_mine && user.subject_id.parse::<i16>().ok()==Some(number);
                    html!("button", {
                        .class("ornament")
                        .class_signal("filled", futures_signals::signal::always(greeting.is_some()))
                        .class_signal("mine", futures_signals::signal::always(mine))
                        .style("left", &format!("{x}%")).style("top",&format!("{y}%"))
                        .attr("aria-label",&format!("Шар номер {number}"))
                        .text(&number.to_string())
                        .event(clone!(state, greeting => move |_| {
                            if let Some(g)=greeting.clone() { state.selected.set(Some(g)); }
                            else if mine { show_form(state.clone()); }
                            else { set_note(&state, "Этот шар пока свободен"); }
                        }))
                    })
                }))})
            })))
        }))
        .child(html!("div", {.class("legend")
            .child(html!("span", {.child(html!("i", {.class("dot")})).text("Свободный шар")}))
            .child(html!("span", {.child(html!("i", {.class("dot").class("filled")})).text("Опубликованное поздравление")}))
        }))
        .child(html!("div", {.class("note").text_signal(state.note.signal_cloned())}))
        .child(html!("footer", {.text("С теплом и верой в маленькие чудеса ✨")}))
        .child_signal(state.selected.signal_cloned().map(clone!(state => move |selected| selected.map(|g| html!("div", {
            .class("overlay").event(clone!(state => move |event: events::Click| { if event.target()==event.current_target(){state.selected.set(None);} }))
            .child(html!("section", {.class("dialog")
                .child(html!("button", {.class("close").text("×").event(clone!(state => move |_| state.selected.set(None)))}))
                .child(html!("div", {.class("kicker").text("CHRISTMAS WISH")}))
                .child(html!("h2", {.text(&format!("С теплом, {}",g.display_name))}))
                .child(html!("p", {.text(&g.message)}))
                .child(html!("button", {.class("primary").text("Вернуться к ёлке").event(clone!(state => move |_| state.selected.set(None)))}))
            }))
        }))))
    })
}
fn show_form(state: AppState) {
    let name=state.name.get_cloned(); let message=state.message.get_cloned();
    let form_state=state.clone();
    let dialog=html!("div", {.class("overlay")
        .child(html!("section", {.class("dialog")
            .child(html!("button", {.class("close").text("×").event(|_| { if let Some(w)=web_sys::window(){let _=w.location().reload();} })))}))
            .child(html!("div", {.class("kicker").text("YOUR PERSONAL ORNAMENT")}))
            .child(html!("h2", {.text("Добавь своё пожелание")}))
            .child(html!("p", {.class("muted").text("Твоё приглашение одноразовое. После публикации поздравление появится у всей команды.")}))
            .child(field("Как тебя зовут?","Твоё имя",state.name.clone(),false))
            .child(field("Твоё поздравление","Пусть в новом году...",state.message.clone(),true))
            .child(html!("div", {.class("note").text_signal(state.note.signal_cloned())}))
            .child(html!("button", {.class("primary").text("Опубликовать поздравление").event(clone!(form_state => move |_| {
                let display_name=form_state.name.get_cloned(); let message=form_state.message.get_cloned();
                let csrf=form_state.csrf(); let s=form_state.clone();
                spawn_local(async move {
                    let body=serde_json::json!({"display_name":display_name,"message":message});
                    match post_json::<serde_json::Value>("/api/greetings",&body,Some(&csrf)).await {
                        Ok(_) => { set_note(&s,"Поздравление опубликовано! ✨"); s.name.set(String::new());s.message.set(String::new());load_greetings(s.clone()).await;if let Some(w)=web_sys::window(){let _=w.location().reload();} }
                        Err(e)=>set_note(&s,e),
                    }
                });
            })))}))
        }))
    });
    if let Some(document)=web_sys::window().and_then(|w|w.document()) {
        if let Some(root)=document.get_element_by_id("app") { append_dom(&root,dialog); }
    }
    let _=(name,message);
}
fn admin_screen(state: AppState) -> Dom {
    html!("main", {.class("shell")
        .child(html!("div", {.class("eyebrow").text("CONGRATIS · CONTROL ROOM")}))
        .child(html!("h1", {.text("Панель ").child(html!("span", {.text("администратора")}))}))
        .child(html!("p", {.class("lead").text("Управление приглашениями и статусом двадцати шаров.")}))
        .child(html!("div", {.class("toolbar")
            .child(html!("button", {.class("secondary").text("К ёлке").event(|_| {if let Some(w)=web_sys::window(){let _=w.location().set_hash("");let _=w.location().reload();}})))}))
            .child(html!("button", {.class("secondary").text("Обновить").event(clone!(state => move |_| {let s=state.clone();spawn_local(async move{load_participants(s).await;});})))}))
        }))
        .child_signal(state.participants.signal_cloned().map(clone!(state => move |participants| {
            html!("div", {.class("admin-grid").children(participants.into_iter().map(clone!(state => move |p| {
                let number=p.ornament_number;
                let status=if p.submitted {"Поздравление опубликовано"} else if p.activated {"Приглашение активировано"} else if p.revoked {"Приглашение отозвано"} else {"Ожидает участника"};
                html!("div", {.class("admin-item")
                    .child(html!("strong", {.text(&format!("Шар №{number}"))}))
                    .child(html!("small", {.text(status)}))
                    .child(html!("small", {.text(p.display_name.unwrap_or_default())}))
                    .children(if !p.activated && !p.submitted && !p.revoked {
                        vec![
                            html!("button", {.class("secondary").text("Перевыпустить ссылку").event(clone!(state => move |_| {
                                let csrf=state.csrf();let s=state.clone();
                                spawn_local(async move {
                                    match post_json::<serde_json::Value>(&format!("/api/admin/invitations/{number}/reissue"),&serde_json::json!({}),Some(&csrf)).await {
                                        Ok(data)=>{let token=data["token"].as_str().unwrap_or_default();let origin=web_sys::window().map(|w|w.location().origin().unwrap_or_default()).unwrap_or_default();let url=format!("{origin}/#invite={token}");s.invite_url.set(url.clone());set_note(&s,format!("Одноразовая ссылка для шара №{number}: {url}"));load_participants(s.clone()).await;}
                                        Err(e)=>set_note(&s,e)
                                    }
                                });
                            })))}),
                            html!("button", {.class("secondary").text("Отозвать").event(clone!(state => move |_| {
                                let csrf=state.csrf();let s=state.clone();
                                spawn_local(async move {
                                    match post_empty(&format!("/api/admin/invitations/{number}/revoke"),&csrf).await {Ok(())=>load_participants(s).await,Err(e)=>set_note(&s,e)}
                                });
                            })))})
                        ]
                    } else { vec![] })
                })
            })))})
        })))
        .child(html!("div", {.class("note").text_signal(state.note.signal_cloned())}))
        .child(html!("footer", {.text("Ссылки-приглашения показываются только один раз. Сохраните их сразу после выпуска.")}))
    })
}
fn admin_login(state: AppState) -> Dom {
    html!("main", {.class("shell")
        .child(html!("div", {.class("eyebrow").text("CONGRATIS · ADMIN")}))
        .child(html!("h1", {.text("Вход в ").child(html!("span", {.text("админку")}))}))
        .child(html!("section", {.class("panel")
            .child(html!("div", {.class("kicker").text("AUTHORIZED ACCESS")}))
            .child(field("Логин","admin",state.admin_name.clone(),false))
            .child(html!("label", {.text("Пароль").child(html!("input", {
                .attr("type","password").attr("placeholder","Пароль администратора")
                .event(clone!(state => move |event: events::Input| {if let Some(target)=event.target(){if let Ok(input)=target.dyn_into::<HtmlInputElement>(){state.admin_password.set(input.value());}}}))
            }))}))
            .child(html!("div", {.class("note").text_signal(state.note.signal_cloned())}))
            .child(html!("button", {.class("primary").text("Войти").event(clone!(state => move |_| {
                let username=state.admin_name.get_cloned();let password=state.admin_password.get_cloned();let s=state.clone();
                spawn_local(async move {
                    match post_json::<User>("/api/auth/admin/login",&serde_json::json!({"username":username,"password":password}),None).await {
                        Ok(user)=>{if let Some(w)=web_sys::window(){let _=w.location().set_hash("/admin");}s.user.set(Some(user));load_participants(s.clone()).await;}
                        Err(e)=>set_note(&s,e)
                    }
                });
            })))}))
        }))
        .child(html!("footer", {.text("Доступ только для администратора.")}))
    })
}
fn render(state: AppState) -> Dom {
    html!("div", {.child_signal(state.user.signal_cloned().map(clone!(state => move |user| {
        if let Some(user)=user {
            if user.role=="admin" {
                let is_admin=web_sys::window().map(|w|w.location().hash().unwrap_or_default().contains("/admin")).unwrap_or(false);
                if is_admin { admin_screen(state.clone()) } else { participant_tree(state.clone()) }
            } else { participant_tree(state.clone()) }
        } else {
            let hash=web_sys::window().map(|w|w.location().hash().unwrap_or_default()).unwrap_or_default();
            if hash.starts_with("#invite=") {
                let token=hash.trim_start_matches("#invite=").to_string();
                if let Some(w)=web_sys::window(){if let Ok(history)=w.history(){let _=history.replace_state_with_url(&wasm_bindgen::JsValue::NULL,"","/");}}
                let s=state.clone();
                spawn_local(async move {
                    match post_json::<User>("/api/auth/participant/exchange",&serde_json::json!({"token":token}),None).await {
                        Ok(user)=>{s.user.set(Some(user));load_greetings(s.clone()).await;connect_events(s);}
                        Err(e)=>set_note(&s,format!("Не удалось активировать приглашение: {e}"))
                    }
                });
                html!("main", {.class("shell").child(html!("h1", {.text("Открываем твоё приглашение…")})).child(html!("p", {.class("note").text_signal(state.note.signal_cloned())}))})
            } else if hash.contains("/admin") { admin_login(state.clone()) }
            else { html!("main", {.class("shell").child(html!("div", {.class("eyebrow").text("A LITTLE CHRISTMAS MAGIC")})).child(html!("h1", {.text("Ёлка добрых ").child(html!("span", {.text("пожеланий")}))})).child(html!("p", {.class("lead").text("Чтобы увидеть общую ёлку, открой персональную ссылку-приглашение.")})).child(html!("button", {.class("primary").text("Вход администратора").event(|_|{if let Some(w)=web_sys::window(){let _=w.location().set_hash("/admin");let _=w.location().reload();}})))}))}) }
        }
    })))})
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
            if user.role=="admin" {load_participants(s.clone()).await;} else {load_greetings(s.clone()).await;connect_events(s.clone());}
        }
    });
}
