// Untrusted content-island code in a Web Worker. No DOM, no window.zetl.
var n = 0;
function paint(){ self.postMessage({op:"render",tree:{tag:"div",props:{class:"ibox"},children:[
  {tag:"p",props:{class:"ilabel"},children:["counter island — painted by a sandboxed Worker, re-rendered each tick:"]},
  {tag:"p",props:{class:"inum"},children:[String(n)]} ]}}); }
self.onmessage=function(e){ if(e.data&&e.data.op==="boot"){ paint();
  self.postMessage({op:"publish",topic:"content:count",value:n});
  setInterval(function(){ n++; paint(); self.postMessage({op:"publish",topic:"content:count",value:n}); },1000);
}};
