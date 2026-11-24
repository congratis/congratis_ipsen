const ornamentsEl = document.getElementById('ornaments');
const greetingModal = document.getElementById('greetingModal');
const formModal = document.getElementById('formModal');
const token = new URLSearchParams(location.search).get('token');
let greetings = [];

function showModal(el){el.classList.add('open');el.setAttribute('aria-hidden','false')}
function hideModal(el){el.classList.remove('open');el.setAttribute('aria-hidden','true')}
document.querySelectorAll('[data-close]').forEach(b=>b.addEventListener('click',()=>{hideModal(greetingModal);hideModal(formModal)}));
[greetingModal,formModal].forEach(m=>m.addEventListener('click',e=>{if(e.target===m)hideModal(m)}));

const positions=[[49,12],[36,20],[62,22],[25,31],[47,29],[72,34],[15,43],[34,42],[56,43],[80,48],[25,55],[45,55],[66,57],[12,66],[34,68],[55,69],[77,68],[23,80],[47,81],[68,81]];
function render(){
  ornamentsEl.innerHTML='';
  positions.forEach(([x,y],i)=>{
    const n=i+1,g=greetings.find(item=>item.ornament_number===n);
    const b=document.createElement('button');
    b.className='ornament '+(g?'filled':'available');
    b.style.left=`calc(${x}% - 18px)`;b.style.top=`${y}%`;
    b.setAttribute('aria-label',g?`Шар ${n}: поздравление от ${g.display_name}`:`Свободный шар ${n}`);
    b.addEventListener('click',()=>{
      document.getElementById('modalTitle').textContent=g?`С теплом, ${g.display_name}`:`Шар №${n}`;
      document.getElementById('modalMessage').textContent=g?g.message:'Этот шар пока ждёт своего поздравления.';
      showModal(greetingModal);
    });
    ornamentsEl.appendChild(b);
  });
}
async function load(){
  try{const r=await fetch('/api/greetings');if(!r.ok)throw new Error();greetings=await r.json();render()}
  catch(e){ornamentsEl.innerHTML='<p style="color:#ffd0a0">Не удалось загрузить ёлку. Обновите страницу.</p>'}
}
function applyGreeting(greeting){
  const index=greetings.findIndex(item=>item.ornament_number===greeting.ornament_number);
  if(index>=0)greetings[index]=greeting;else greetings.push(greeting);
  render();
}
function connectRealtime(){
  if(!('EventSource' in window))return;
  const source=new EventSource('/api/events');
  source.addEventListener('greeting',event=>{
    try{applyGreeting(JSON.parse(event.data))}catch(e){console.warn('Invalid greeting event',e)}
  });
  source.addEventListener('resync',()=>load());
  // EventSource fires onopen after the initial connection and every reconnect.
  // A fresh snapshot repairs any events missed while the browser was offline.
  source.onopen=()=>load();
connectRealtime();
}
if(token){document.getElementById('greetingForm').dataset.token=token;showModal(formModal)}
document.getElementById('greetingForm').addEventListener('submit',async e=>{
  e.preventDefault();
  const form=e.currentTarget,note=document.getElementById('formNote'),button=form.querySelector('button[type=submit]');
  button.disabled=true;note.textContent='Сохраняем пожелание…';
  const data=Object.fromEntries(new FormData(form).entries());data.token=form.dataset.token;
  try{
    const r=await fetch('/api/greetings',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(data)});
    const body=await r.json();if(!r.ok)throw new Error(body.message||'Не удалось сохранить поздравление');
    note.textContent='Готово! Твоё пожелание уже на ёлке ✨';form.reset();history.replaceState({},'',location.pathname);
    await load();setTimeout(()=>hideModal(formModal),1300);
  }catch(err){note.textContent=err.message;button.disabled=false}
});
load();
