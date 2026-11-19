import { tools } from './work/mcp/tools.ts';
import { operation, buildRequest, registerTools } from './work/mcp/_/tool.ts';
import { createRequire } from 'node:module';
import { readFileSync, writeFileSync } from 'node:fs';
const require=createRequire(import.meta.url);
const doc=JSON.parse(readFileSync(new URL('../../contracts/v1/openapi.json',import.meta.url)));
const assertions=[];
for(const tool of tools){
 const op=operation(tool);const canonical=doc.paths[op.path].post;
 if(tool.name!==canonical.operationId)throw Error('operation drift');
 if(!op.responses.schemas['422'] || !op.responses.schemas['200'])throw Error('error/success coverage');
 const request=buildRequest(tool,{body:{input:{path:'one.pdf'}}},'http://127.0.0.1');
 if(request.method!=='POST'||JSON.parse(request.body).input.path!=='one.pdf')throw Error('request codec');
}
const extract=tools.find(t=>t.name==='extractPdf');
const input=extract.inputSchema.$defs.ExtractRequest;
assertions.push({finding:'additionalProperties dropped from request objects',observed:input.additionalProperties===undefined});
assertions.push({finding:'uniqueItems dropped from page selection',observed:input.properties.pages.uniqueItems===undefined});
assertions.push({finding:'numeric minimum dropped',observed:input.properties.pages.items.minimum===undefined});
assertions.push({finding:'string min/max lengths dropped',observed:extract.inputSchema.$defs.Input.properties.path.minLength===undefined});
const registered=[];registerTools({registerTool:(...args)=>registered.push(args)},tools,{baseUrl:'http://127.0.0.1'});
const schema=registered[0][1].outputSchema;
const invalid=schema['~standard'].validate({invalid:true});
assertions.push({finding:'upstream output validator accepts invalid output',observed:!!invalid.value});
for(const a of assertions)if(!a.observed)throw Error('review fidelity probe: '+a.finding);
console.log(JSON.stringify({operations:tools.length,requestCodec:'passed',schemaFidelity:'gaps',findings:assertions},null,2));
