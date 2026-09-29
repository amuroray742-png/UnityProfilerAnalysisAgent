// Persistent optimization view DTOs; backup bytes remain in Rust.
export interface OptimizationTask {id:string;kind:string;title:string;evidence:string;files:Record<string,string>;instructions:string;acceptance:string;constraints:string;selected:boolean}
export interface Conditions {device:string;platform:string;scenario:string;operation:string;build:string;quality:string;resolution:string;profiling:string;codeVersion:string}
export interface Capture {id:string;path:string;conditions:Conditions;snapshot:{fileName:string;frameCount:number;unityVersion:string|null}}
export interface Run {id:string;agentId:string;sessionId:string;status:string;reason:string|null;createdAt:string;checks:{status:string;reason?:string;checkedRevision?:number}[];changes:{path:string;state:string;beforeHash:string;afterHash:string}[]}
export interface Round {performanceStatus:string;taskVersion:number;taskVerifications:Record<string,string>;id:string;baseline:string;candidate:string|null;tasks:OptimizationTask[];tests:string[];runs:Run[];comparison:Comparison|null;correctness:string;decision:string;reports:{agentId:string;stage:string}[]}
export type Statistic = 'mean'|'p50'|'p95'|'p99'|'max';
export interface Stat {mean:number|null;p50:number|null;p95:number|null;p99:number|null;max:number|null;validFrames:number;totalFrames:number}
export interface HotspotPage {rows:{roleAndPath:[string|null,string,string[]];a:{inclusiveMsPerFrame:number;callsPerFrame:number}|null;b:{inclusiveMsPerFrame:number;callsPerFrame:number}|null;deltaInclusiveMsPerFrame:number|null;association:string}[];nextStart:number|null;total:number;status?:string;reason?:string}
export interface Comparison {baseline:string;candidate:string;hotspots:HotspotPage;comparability:string;scope:string;unknown:string[];mismatch:string[];metrics:{metric:string;a:Stat;b:Stat;delta:Record<Statistic,{absolute:number|null;percent:number|null}>;verdict:string}[]}
export interface Project {id:string;name:string;root:string;directory:string;busy:boolean;budgets:Record<string,number>;captures:Capture[];rounds:Round[]}
