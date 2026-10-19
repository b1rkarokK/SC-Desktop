import {readFileSync,writeFileSync} from 'node:fs';
function ed(p,f){let s=readFileSync(p,'utf8');const crlf=s.includes('\r\n');s=s.replace(/\r\n/g,'\n');s=f(s);writeFileSync(p,crlf?s.replace(/\n/g,'\r\n'):s);}
// 1.4.7 was not released on its own: its notes are part of 1.4.8
const cut=(s,tag)=>{const m=new RegExp(`## \[${tag}\][^\n]*\n\n([\s\S]*?)(?=## \[)`).exec(s);return m?m[1].trim():''};
for(const f of ['CHANGELOG.md','CHANGELOG.en.md']) ed(f,s=>{const body=cut(s,'1\.4\.7');return s.replace(/(## \[1\.4\.8\][^\n]*\n\n[\s\S]*?)(?=\n## \[1\.4\.7\])/,(m)=>m.trimEnd()+'\n'+body+'\n').replace(/## \[1\.4\.7\][^\n]*\n\n[\s\S]*?(?=## \[1\.4\.6\])/,'');});
