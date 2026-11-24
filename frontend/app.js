const ornamentsEl = document.getElementById('ornaments');
const greetingModal = document.getElementById('greetingModal');
const formModal = document.getElementById('formModal');
const token = new URLSearchParams(location.search).get('token') || new URLSearchParams(location.hash.slice(1)).get('invite');
const accessToken = new URLSearchParams(location.hash.slice(1)).get('access');
const treeContent = document.getElementById('treeContent');
const accessPanel = document.getElementById('accessPanel');
const accessMessage = document.getElementById('accessMessage');
let greetings = [];
let realtimeSequence = 0;
let realtimeEvents = [];
let loadRequestId = 0;

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
function applyRealtimeEvent(event){
  const byNumber=new Map(greetings.map(item=>[item.ornament_number,item]));
  if(event.type==='greeting')byNumber.set(event.payload.ornament_number,event.payload);
  if(event.type==='removed')byNumber.delete(event.payload.ornament_number);
  greetings=[...byNumber.values()];
  render();
}
function recordRealtimeEvent(type,payload){
  const event={sequence:++realtimeSequence,type,payload};
  realtimeEvents.push(event);
  applyRealtimeEvent(event);
}
async function load(){
  const requestId=++loadRequestId;
  const sequenceAtStart=realtimeSequence;
  try{
    const r=await fetch('/api/greetings');if(!r.ok)throw new Error();
    const snapshot=await r.json();
    if(requestId!==loadRequestId)return;
    const merged=new Map(snapshot.map(item=>[item.ornament_number,item]));
    realtimeEvents
      .filter(event=>event.sequence>sequenceAtStart)
      .forEach(event=>{
        if(event.type==='greeting')merged.set(event.payload.ornament_number,event.payload);
        if(event.type==='removed')merged.delete(event.payload.ornament_number);
      });
    greetings=[...merged.values()];
    realtimeEvents=[];
    render();
  }
  catch(e){
    if(requestId===loadRequestId)ornamentsEl.innerHTML='<p style="color:#ffd0a0">Не удалось загрузить ёлку. Обновите страницу.</p>';
  }
}
function connectRealtime(){
  if(!('EventSource' in window)){load();return}
  const source=new EventSource('/api/events');
  source.addEventListener('greeting',event=>{
    try{recordRealtimeEvent('greeting',JSON.parse(event.data))}catch(e){console.warn('Invalid greeting event',e)}
  });
  source.addEventListener('ornament_removed',event=>{
    try{
      recordRealtimeEvent('removed',JSON.parse(event.data));
    }catch(e){console.warn('Invalid removal event',e)}
  });
  source.addEventListener('resync',()=>load());
  // Fetch after the stream is open to close the snapshot/subscribe race.
  // EventSource fires onopen again after every automatic reconnect.
  source.onopen=()=>load();
}
if(token){document.getElementById('greetingForm').dataset.token=token;}
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
async function initializeParticipantView(){
  accessPanel.hidden=false;
  treeContent.hidden=true;
  accessMessage.textContent='Проверяем доступ…';
  try{
    if(accessToken){
      const exchange=await fetch('/api/participant/exchange',{
        method:'POST',
        headers:{'Content-Type':'application/json'},
        credentials:'same-origin',
        body:JSON.stringify({token:accessToken})
      });
      const exchangeBody=await exchange.json().catch(()=>({}));
      if(!exchange.ok)throw new Error(exchangeBody.message||'Персональная ссылка недействительна');
      history.replaceState({},'',location.pathname+location.search);
    }
    const session=await fetch('/api/participant/session',{credentials:'same-origin'});
    if(!session.ok)throw new Error('Эта ёлка доступна только участникам. Откройте персональную ссылку, выданную администратором.');
    accessPanel.hidden=true;
    treeContent.hidden=false;
    if(token){history.replaceState({},'',location.pathname);showModal(formModal);}
    connectRealtime();
  }catch(error){
    accessMessage.textContent=error.message;
  }
}
initializeParticipantView();
