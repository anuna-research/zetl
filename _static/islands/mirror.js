// A DIFFERENT sandboxed worker. It only hears the counter via the shell bus — the two
// workers cannot reach each other directly (REQ-5004/5016).
var last = "—";
function paint(){ self.postMessage({op:"render",tree:{tag:"div",props:{class:"ibox b2"},children:[
  {tag:"p",props:{class:"ilabel"},children:["mirror island — a separate Worker, subscribed to content:count via the bus:"]},
  {tag:"p",props:{class:"inum"},children:["heard "+last]} ]}}); }
self.onmessage=function(e){ var m=e.data; if(!m)return;
  if(m.op==="boot"){ paint(); self.postMessage({op:"subscribe",topic:"content:count"}); }
  else if(m.op==="update"&&m.topic==="content:count"){ last=String(m.value)+" (seq "+m.seq+")"; paint(); }
};
