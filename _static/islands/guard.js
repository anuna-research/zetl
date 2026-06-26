// This worker is hostile on purpose. The trusted host drops every dangerous thing.
self.onmessage=function(e){ if(e.data&&e.data.op==="boot"){
  self.postMessage({op:"render",tree:{tag:"div",props:{class:"ibox b3"},children:[
    {tag:"p",props:{class:"ilabel"},children:["guard island — this Worker tries to attack; the host strips each one:"]},
    {tag:"script",props:{},children:["window.__pwned=1"]},                         // dropped (forbidden tag)
    {tag:"iframe",props:{src:"https://evil.example"},children:[]},                  // dropped (forbidden tag)
    {tag:"a",props:{href:"javascript:alert(1)"},children:["javascript: link (href stripped)"]},
    {tag:"img",props:{src:"https://tracker.example/x.gif"}},                        // src dropped (remote, CON-5007)
    {tag:"p",props:{onclick:"steal()"},children:["a paragraph (its onclick is stripped)"]},
    {tag:"p",props:{class:"iblocked"},children:["✓ script, iframe, javascript: href, remote img and on* were all removed by the host renderer."]}
  ]}});
  self.postMessage({op:"publish",topic:"theme",value:"dark"});      // DENIED: trusted topic, ungranted
  self.postMessage({op:"publish",topic:"content:ok",value:"maybe"}); // DENIED: not in enum
  self.postMessage({op:"publish",topic:"content:ok",value:"yes"});   // allowed
}};
