// BM25 full-text scorer — SPEC-013 §4.9  (k1=1.2, b=0.75)
// search(query, index) where index = {avgDl, docs:[{n,s,dl,tf}], df}
// Returns [{n,s,score}] sorted descending, score > 0.
function bm25Search(query, index) {
  var k1 = 1.2, b = 0.75;
  var N = index.docs.length;
  var avgDl = index.avgDl || 1;
  var terms = query.toLowerCase().split(/[^a-z0-9]+/).filter(function(t) { return t.length > 0; });
  if (!terms.length) return [];
  var results = [];
  for (var i = 0; i < N; i++) {
    var doc = index.docs[i];
    var dl = doc.dl || 0;
    var score = 0;
    for (var j = 0; j < terms.length; j++) {
      var term = terms[j];
      var tf = (doc.tf && doc.tf[term]) || 0;
      if (!tf) continue;
      var df = (index.df && index.df[term]) || 0;
      var idf = Math.log((N - df + 0.5) / (df + 0.5) + 1);
      var tfNorm = (tf * (k1 + 1)) / (tf + k1 * (1 - b + b * dl / avgDl));
      score += idf * tfNorm;
    }
    if (score > 0) results.push({ n: doc.n, s: doc.s, score: score });
  }
  results.sort(function(a, b) { return b.score - a.score; });
  return results;
}
