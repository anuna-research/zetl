self.onmessage=function(e){ if(e.data&&e.data.op==="boot"){
  var t=new Date().toLocaleTimeString();
  self.postMessage({op:"render",tree:{tag:"div",props:{class:"ibox b4"},children:[
    {tag:"p",props:{class:"ilabel"},children:["lazy island (hydrate=visible) — its Worker spawned only when you scrolled here:"]},
    {tag:"p",props:{class:"inum"},children:["mounted "+t]} ]}});
  self.postMessage({op:"publish",topic:"content:woke",value:"yes"});
}};
